// ===================================================================
// mcu_ctrl — the vendor control interface, CRC and status.
//
// A byte stream in each direction over the "smartcard-ctl" vendor interface,
// records `type(1) | len(2 BE) | payload`:
//
//   host -> fpga : 01 LOAD off(2 BE) data, 02 ENABLE en(1), 03 STATUS_REQ,
//                  04 CRC_REQ
//   fpga -> host : 81 NOTICE off(4 BE) len(2 BE) latency(4 BE),
//                  82 STATUS loaded(1) enabled(1) answered(4 BE) triggers(4 BE),
//                  83 CRC crc32(4 BE), 84 ERROR code(1)
//
// The image and the enable PERSIST across DTR power-cycles and ATRs; only a
// USB bus reset or a new LOAD clears them. The answered/triggers counters are
// per-image: a new LOAD (and a USB reset) zeroes them, so STATUS reads a
// per-session total rather than a lifetime one. A LOAD while the card is running is
// refused (0x84 code 0x01) so the image is never torn under a live read. The
// CRC is zlib crc32 over the whole 32 KB (init/xorout 0xFFFFFFFF, poly
// 0xEDB88320), which `ledger_console` compares against crc32fast of the
// 0xff-padded image. CHECKED in smartcard_mcu_ctrl_tb.v.
// ===================================================================
module mcu_ctrl (
    input  wire        clk,
    input  wire        rst_n,
    input  wire        usb_reset,
    input  wire        running,       // card powered and out of reset

    // Vendor OUT: records from the host.
    input  wire [7:0]  vout_data,
    input  wire        vout_valid,
    output wire        vout_ready,

    // Vendor IN: records to the host.
    output reg  [7:0]  vin_data,
    output reg         vin_valid,
    input  wire        vin_ready,
    output reg         vin_commit,

    // The image write port.
    output reg         img_wr,
    output reg  [14:0] img_wr_addr,
    output reg  [7:0]  img_wr_data,

    // The image read port for the CRC walk (the responder has priority). The
    // address is combinational on the walk counter so the registered read has
    // a whole settle cycle (C_SET) before the byte is accumulated (C_ACC).
    output wire [14:0] crc_rd_addr,
    input  wire [7:0]  crc_rd_data,
    input  wire        resp_busy,

    output reg         enabled,
    output reg         loaded,

    // From the responder.
    input  wire        trig_stb,
    input  wire        answered_stb,
    input  wire [79:0] notice,
    input  wire        notice_stb
);
    localparam [7:0] R_LOAD = 8'h01, R_ENABLE = 8'h02, R_STATUS = 8'h03, R_CRC = 8'h04;
    localparam [7:0] T_NOTICE = 8'h81, T_STATUS = 8'h82, T_CRC = 8'h83, T_ERROR = 8'h84;
    localparam [7:0] ERR_CARD_ON = 8'h01, ERR_PROTOCOL = 8'h02;

    // Always ready to take the next OUT byte: records are parsed as they
    // arrive and the writes they drive keep up with the bulk endpoint.
    assign vout_ready = 1'b1;

    // ---- counters and flags, persistent across DTR/ATR ----
    reg [31:0] answered;
    reg [31:0] triggers;
    reg [31:0] crc_val;

    // ---- the OUT record parser ----
    localparam [2:0] P_TYPE = 3'd0, P_LENH = 3'd1, P_LENL = 3'd2, P_PAY = 3'd3;
    reg [2:0]  pstate;
    reg [7:0]  rtype;
    reg [7:0]  rlenh;
    reg [15:0] rlen;
    reg [15:0] rpos;
    reg [15:0] load_off;   // off(2 BE) of a LOAD record
    reg        load_reject;

    wire pay_byte = (pstate == P_PAY) && vout_valid;
    wire pay_last = pay_byte && (rpos == rlen - 16'd1);

    // ---- outgoing record requests ----
    reg        req_status;
    reg        req_crc;      // the CRC walk should (re)start
    reg        req_err;
    reg [7:0]  err_code;
    reg        crc_ready;    // a fresh CRC value is waiting to be sent

    // notice FIFO: {off[31:0], len[15:0], latency[31:0]}.
    wire        nf_empty, nf_full;
    wire [79:0] nf_rd;
    reg         nf_rd_en;
    fifo_sync #(.WIDTH(80), .DEPTH(16), .FWFT(1), .ADDR_WIDTH(4), .CNT_WIDTH(5)) nfifo (
        .clk(clk), .rst_n(rst_n),
        .wr_en(notice_stb & ~nf_full), .wr_data(notice), .full(nf_full),
        .rd_en(nf_rd_en), .rd_data(nf_rd), .empty(nf_empty), .count()
    );

    // ---- the CRC walk ----
    localparam [1:0] C_IDLE = 2'd0, C_SET = 2'd1, C_ACC = 2'd2, C_FIN = 2'd3;
    reg [1:0]  cstate;
    reg [15:0] caddr;        // 0 .. 0x7FFF
    reg [31:0] crc_acc;
    assign crc_rd_addr = caddr[14:0];

    // One byte of a reflected CRC32, eight steps unrolled combinationally.
    function [31:0] crc32_byte;
        input [31:0] c_in;
        input [7:0]  d;
        integer      k;
        reg [31:0]   c;
        begin
            c = c_in ^ {24'd0, d};
            for (k = 0; k < 8; k = k + 1)
                c = (c >> 1) ^ (32'hEDB88320 & {32{c[0]}});
            crc32_byte = c;
        end
    endfunction

    // ---- the OUT -> IN record output FSM ----
    localparam [1:0] O_IDLE = 2'd0, O_SEND = 2'd1;
    reg [1:0]  ostate;
    reg [7:0]  osel;         // which record is being sent (its type byte)
    reg [3:0]  oidx;         // byte position in the record
    reg [3:0]  olast;        // index of the record's final byte
    // The record's payload, latched at the start of a send.
    reg [79:0] o_pay;        // up to 10 bytes, byte 0 in the high byte
    reg [7:0]  o_plen;

    // The byte at position `oidx` of the record being sent: type, then the two
    // length bytes, then the payload held MSB-first in `o_pay`.
    function [7:0] out_byte;
        input [7:0]  sel;
        input [3:0]  idx;
        input [7:0]  plen;
        input [79:0] pay;
        begin
            if (idx == 4'd0)      out_byte = sel;
            else if (idx == 4'd1) out_byte = 8'd0;
            else if (idx == 4'd2) out_byte = plen;
            else                  out_byte = pay[(10 - (idx - 4'd3)) * 8 - 1 -: 8];
        end
    endfunction

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            pstate <= P_TYPE; rtype <= 8'd0; rlenh <= 8'd0; rlen <= 16'd0;
            rpos <= 16'd0; load_off <= 16'd0; load_reject <= 1'b0;
            enabled <= 1'b0; loaded <= 1'b0;
            answered <= 32'd0; triggers <= 32'd0; crc_val <= 32'd0;
            img_wr <= 1'b0; img_wr_addr <= 15'd0; img_wr_data <= 8'd0;
            req_status <= 1'b0; req_crc <= 1'b0; req_err <= 1'b0; err_code <= 8'd0;
            crc_ready <= 1'b0;
            nf_rd_en <= 1'b0;
            cstate <= C_IDLE; caddr <= 16'd0; crc_acc <= 32'd0;
            ostate <= O_IDLE; osel <= 8'd0; oidx <= 4'd0; olast <= 4'd0;
            o_pay <= 80'd0; o_plen <= 8'd0;
            vin_data <= 8'd0; vin_valid <= 1'b0; vin_commit <= 1'b0;
        end else begin
            img_wr   <= 1'b0;
            nf_rd_en <= 1'b0;

            // Counters from the responder.
            if (trig_stb)     triggers <= triggers + 32'd1;
            if (answered_stb) answered <= answered + 32'd1;

            // A USB bus reset clears the image's state (not a DTR drop).
            if (usb_reset) begin
                enabled  <= 1'b0;
                loaded   <= 1'b0;
                answered <= 32'd0;
                triggers <= 32'd0;
            end

            // ---------------- OUT record parser ----------------
            if (vout_valid) begin
                case (pstate)
                    P_TYPE: begin rtype <= vout_data; pstate <= P_LENH; end
                    P_LENH: begin rlenh <= vout_data; pstate <= P_LENL; end
                    P_LENL: begin
                        rlen <= {rlenh, vout_data};
                        rpos <= 16'd0;
                        // Decide a LOAD's fate before any byte is written.
                        load_reject <= (rtype == R_LOAD) && running;
                        if ({rlenh, vout_data} == 16'd0) begin
                            // An empty record: act on it now.
                            case (rtype)
                                R_STATUS: req_status <= 1'b1;
                                R_CRC:    req_crc    <= 1'b1;
                                R_LOAD, R_ENABLE: begin end  // malformed, ignored
                                default: begin req_err <= 1'b1; err_code <= ERR_PROTOCOL; end
                            endcase
                            pstate <= P_TYPE;
                        end else begin
                            pstate <= P_PAY;
                        end
                    end
                    default: begin   // P_PAY
                        case (rtype)
                            R_LOAD: begin
                                if (rpos == 16'd0) load_off[15:8] <= vout_data;
                                if (rpos == 16'd1) load_off[7:0]  <= vout_data;
                                if (rpos >= 16'd2 && !load_reject) begin
                                    img_wr      <= 1'b1;
                                    // The image is 32 KB, so the address is 15
                                    // bits; `load_off` is a byte offset the
                                    // host keeps below 0x8000.
                                    img_wr_addr <= load_off[14:0] + (rpos[14:0] - 15'd2);
                                    img_wr_data <= vout_data;
                                end
                            end
                            R_ENABLE: if (rpos == 16'd0) enabled <= (vout_data != 8'd0);
                            default: ;
                        endcase
                        rpos <= rpos + 16'd1;
                        if (rpos == rlen - 16'd1) begin
                            // Record end.
                            if (rtype == R_LOAD) begin
                                if (load_reject) begin
                                    req_err  <= 1'b1;
                                    err_code <= ERR_CARD_ON;
                                end else begin
                                    loaded   <= 1'b1;
                                    enabled  <= 1'b0;   // a new LOAD clears enable
                                    // A new image is a new session: the answer
                                    // and trigger counts start from zero, so
                                    // STATUS reads per-image rather than a
                                    // lifetime total (the card is off during a
                                    // LOAD, so nothing is answering to race it).
                                    answered <= 32'd0;
                                    triggers <= 32'd0;
                                end
                            end
                            pstate <= P_TYPE;
                        end
                    end
                endcase
            end

            // ---------------- CRC walk ----------------
            case (cstate)
                C_IDLE: if (req_crc) begin
                    req_crc <= 1'b0;
                    caddr   <= 16'd0;
                    crc_acc <= 32'hFFFF_FFFF;
                    cstate  <= C_SET;
                end
                C_SET: if (!resp_busy) cstate <= C_ACC;   // one settle cycle, addr = caddr
                C_ACC: if (resp_busy) begin
                    cstate <= C_SET;                       // preempted: settle again
                end else begin
                    crc_acc <= crc32_byte(crc_acc, crc_rd_data);
                    if (caddr == 16'h7FFF) begin
                        cstate <= C_FIN;
                    end else begin
                        caddr  <= caddr + 16'd1;
                        cstate <= C_SET;
                    end
                end
                default: begin   // C_FIN
                    crc_val   <= crc_acc ^ 32'hFFFF_FFFF;
                    crc_ready <= 1'b1;
                    cstate    <= C_IDLE;
                end
            endcase

            // ---------------- IN record output FSM ----------------
            case (ostate)
                O_IDLE: begin
                    vin_valid  <= 1'b0;
                    vin_commit <= 1'b0;
                    // Priority: errors, then status, then CRC, then notices.
                    if (req_err) begin
                        req_err <= 1'b0;
                        osel    <= T_ERROR; o_plen <= 8'd1; olast <= 4'd3;
                        o_pay   <= {err_code, 72'd0};
                        oidx    <= 4'd0; ostate <= O_SEND;
                    end else if (req_status) begin
                        req_status <= 1'b0;
                        osel  <= T_STATUS; o_plen <= 8'd10; olast <= 4'd12;
                        o_pay <= {{7'd0, loaded}, {7'd0, enabled}, answered, triggers};
                        oidx  <= 4'd0; ostate <= O_SEND;
                    end else if (crc_ready) begin
                        crc_ready <= 1'b0;
                        osel  <= T_CRC; o_plen <= 8'd4; olast <= 4'd6;
                        o_pay <= {crc_val, 48'd0};
                        oidx  <= 4'd0; ostate <= O_SEND;
                    end else if (!nf_empty) begin
                        // {off[31:0], len[15:0], latency[31:0]} -> 10 payload bytes.
                        nf_rd_en <= 1'b1;
                        osel  <= T_NOTICE; o_plen <= 8'd10; olast <= 4'd12;
                        o_pay <= nf_rd;
                        oidx  <= 4'd0; ostate <= O_SEND;
                    end
                end
                default: begin   // O_SEND, a byte at a time through the IN handshake
                    if (!vin_valid) begin
                        vin_data   <= out_byte(osel, oidx, o_plen, o_pay);
                        vin_valid  <= 1'b1;
                        vin_commit <= (oidx == olast);
                    end else if (vin_ready) begin
                        vin_valid  <= 1'b0;
                        vin_commit <= 1'b0;
                        if (oidx == olast) begin
                            ostate <= O_IDLE;
                        end else begin
                            oidx <= oidx + 4'd1;
                        end
                    end
                end
            endcase
        end
    end
endmodule
