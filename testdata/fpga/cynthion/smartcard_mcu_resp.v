// ===================================================================
// mcu_flash_resp — the in-fabric MCU flash responder.
//
// It reassembles the element's SEPROXYHAL packets from the receive byte
// stream (`tag | length(2 BE) | payload`), and when it sees
// UNSEC_CHUNK_READ_EXT kind 2 (tag 0x33, length 7, payload LL(2 BE) 02
// OFF(4 BE)) **immediately followed** by the end-of-turn status 60 00 02 00
// 00, it injects the element's answer toward the card: 12 | LL'(2 BE) |
// image[OFF .. OFF+LL'], with LL' = min(LL, 0x8000 - OFF).
//
// Framing is reset whenever the card is not running, which covers both a DTR
// drop and the start of each new ATR; it needs no other reset. The element's
// bytes before SEPROXYHAL begins — the ATR and the PPS echo — would otherwise
// desynchronise the parser, so, like ledger_console's own framer, a byte that
// cannot start an element packet (not 0x30..0x6f) is dropped to resynchronise,
// and a length larger than any real element packet (the ATR's 0x3B tag claims
// 0x9098 bytes) is treated as the same desync. CHECKED in
// smartcard_mcu_resp_tb.v; the byte formats are QUOTED from ledger_console.
// ===================================================================
module mcu_flash_resp #(
    // The largest plausible element->terminal packet. Real ones are tiny; the
    // cap exists only to shed the ATR, whose first byte is an element-tag
    // value with an implausible length behind it.
    parameter integer LEN_CAP = 2048
) (
    input  wire        clk,
    input  wire        rst_n,
    input  wire        running,       // card powered and out of reset

    input  wire [7:0]  rx_data,       // the element's bytes
    input  wire        rx_valid,

    input  wire        enabled,       // answer kind-2 reads
    input  wire        loaded,        // an image is present

    output reg  [14:0] rd_addr,       // image read, the responder's port
    output wire        rd_en,
    input  wire [7:0]  rd_data,

    output reg  [7:0]  tx_data,       // toward the card, ahead of the host FIFO
    output reg         tx_valid,
    output reg         tx_active,
    input  wire        tx_ready,      // the UART took the byte this cycle

    output reg         trig_stb,      // one cycle per detected kind-2 trigger
    output reg         answered_stb,  // one cycle per answer actually sent
    output wire        busy,

    output reg  [79:0] notice,        // {off[31:0], len[15:0], latency[31:0]}
    output reg         notice_stb
);
    // ---- the framer ----
    localparam [1:0] F_TAG = 2'd0, F_LENH = 2'd1, F_LENL = 2'd2, F_BODY = 2'd3;
    reg [1:0]  fstate = F_TAG;
    reg [7:0]  ftag   = 8'd0;
    reg [7:0]  flenh  = 8'd0;
    reg [15:0] flen   = 16'd0;
    reg [15:0] fleft  = 16'd0;
    reg [15:0] fpos   = 16'd0;
    reg        fbad   = 1'b0;   // this packet overran the length cap: ignore it

    // The payload bytes any trigger depends on: 0x33's LL/kind/OFF and 0x60's
    // first two status bytes.
    reg [7:0]  b0, b1, b2, b3, b4, b5, b6;

    wire is_se_tag = (rx_data >= 8'h30) && (rx_data <= 8'h6f);
    wire [15:0] len_at_lenl = {flenh, rx_data};

    // A packet has just completed this cycle.
    wire pkt_end_empty = rx_valid && fstate == F_LENL && len_at_lenl == 16'd0;
    wire pkt_end_body  = rx_valid && fstate == F_BODY && fleft == 16'd1;
    wire pkt_end       = running && !fbad && (pkt_end_empty || pkt_end_body);
    // Its tag and length as they are at completion.
    wire [7:0]  e_tag = ftag;
    wire [15:0] e_len = pkt_end_empty ? 16'd0 : flen;

    // A completed 0x33 kind-2 read, and the end-of-turn status. The last
    // payload byte of each is the one arriving this cycle (`rx_data`), not a
    // register: the status's second byte and the read's low offset byte.
    wire is_k2  = pkt_end && (e_tag == 8'h33) && (e_len == 16'd7) && (b2 == 8'h02);
    wire is_sta = pkt_end && (e_tag == 8'h60) && (e_len == 16'd2)
                          && (b0 == 8'h00) && (rx_data == 8'h00);

    reg        pending_k2;          // a kind-2 read is waiting for its status
    reg [15:0] k2_ll;
    reg [31:0] k2_off;

    // The trigger: a kind-2 read confirmed by the status that follows it.
    wire trigger = is_sta && pending_k2;

    // LL' = min(LL, 0x8000 - OFF), clamped to zero past the image.
    wire        off_over  = (k2_off[31:15] != 17'd0);
    wire [15:0] avail     = off_over ? 16'd0 : (16'h8000 - {1'b0, k2_off[14:0]});
    wire [15:0] llp       = (k2_ll < avail) ? k2_ll : avail;

    // ---- the answer ----
    localparam [1:0] A_IDLE = 2'd0, A_HEAD = 2'd1, A_DATA = 2'd2, A_DONE = 2'd3;
    reg [1:0]  astate = A_IDLE;
    reg [1:0]  hidx;                // 0,1,2 of the three header bytes
    reg [15:0] didx;                // which image byte, 0 .. llp-1
    reg [15:0] a_llp;               // LL' of the answer in flight
    reg [14:0] a_off;               // OFF[14:0] for addressing
    reg [31:0] a_off_full;          // OFF for the notice
    reg        prep;                // one cycle for the image read to settle
    reg [31:0] lat;                 // cycles since the trigger
    reg        lat_done;            // the first answer byte has gone

    assign busy  = (astate != A_IDLE);
    assign rd_en = (astate == A_DATA);

    wire [7:0] head_byte = (hidx == 2'd0) ? 8'h12
                         : (hidx == 2'd1) ? a_llp[15:8]
                                          : a_llp[7:0];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            fstate <= F_TAG; ftag <= 8'd0; flenh <= 8'd0; flen <= 16'd0;
            fleft <= 16'd0; fpos <= 16'd0; fbad <= 1'b0;
            b0 <= 8'd0; b1 <= 8'd0; b2 <= 8'd0; b3 <= 8'd0;
            b4 <= 8'd0; b5 <= 8'd0; b6 <= 8'd0;
            pending_k2 <= 1'b0; k2_ll <= 16'd0; k2_off <= 32'd0;
            astate <= A_IDLE; hidx <= 2'd0; didx <= 16'd0;
            a_llp <= 16'd0; a_off <= 15'd0; a_off_full <= 32'd0;
            prep <= 1'b0; lat <= 32'd0; lat_done <= 1'b0;
            rd_addr <= 15'd0;
            tx_data <= 8'd0; tx_valid <= 1'b0; tx_active <= 1'b0;
            trig_stb <= 1'b0; answered_stb <= 1'b0;
            notice <= 80'd0; notice_stb <= 1'b0;
        end else begin
            trig_stb     <= 1'b0;
            answered_stb <= 1'b0;
            notice_stb   <= 1'b0;

            // ---- framing ----
            if (!running) begin
                // The card is off or in reset: hold the framer idle. This is
                // the reset on a DTR drop and on every new ATR.
                fstate     <= F_TAG;
                fbad       <= 1'b0;
                pending_k2 <= 1'b0;
            end else if (rx_valid) begin
                case (fstate)
                    F_TAG: begin
                        fbad <= 1'b0;
                        if (is_se_tag) begin
                            ftag   <= rx_data;
                            fstate <= F_LENH;
                        end
                        // else: not an element tag — dropped, stay in F_TAG.
                    end
                    F_LENH: begin flenh <= rx_data; fstate <= F_LENL; end
                    F_LENL: begin
                        flen  <= len_at_lenl;
                        fleft <= len_at_lenl;
                        fpos  <= 16'd0;
                        if (len_at_lenl == 16'd0) begin
                            fstate <= F_TAG;             // empty packet
                        end else if (len_at_lenl > LEN_CAP[15:0]) begin
                            // Implausible: the ATR, read as a packet. Drop it
                            // and resynchronise from the next byte.
                            fstate <= F_TAG;
                            fbad   <= 1'b1;
                        end else begin
                            fstate <= F_BODY;
                        end
                    end
                    default: begin   // F_BODY
                        if (fpos == 16'd0) b0 <= rx_data;
                        if (fpos == 16'd1) b1 <= rx_data;
                        if (fpos == 16'd2) b2 <= rx_data;
                        if (fpos == 16'd3) b3 <= rx_data;
                        if (fpos == 16'd4) b4 <= rx_data;
                        if (fpos == 16'd5) b5 <= rx_data;
                        if (fpos == 16'd6) b6 <= rx_data;
                        fpos  <= fpos + 16'd1;
                        fleft <= fleft - 16'd1;
                        if (fleft == 16'd1) fstate <= F_TAG;
                    end
                endcase
            end

            // Note what a completed packet means. `fbad` packets never reach
            // pkt_end, so they neither trigger nor clear a pending read.
            if (pkt_end) begin
                if (is_k2) begin
                    // b0..b6 of a 0x33 are the last byte (b6) in this very
                    // cycle; take LL and OFF from the registered bytes plus
                    // the arriving one.
                    pending_k2 <= 1'b1;
                    k2_ll      <= {b0, b1};
                    k2_off     <= {b3, b4, b5, rx_data};
                end else if (is_sta && pending_k2) begin
                    pending_k2 <= 1'b0;
                    trig_stb   <= 1'b1;
                    if (enabled && loaded && astate == A_IDLE) begin
                        astate     <= A_HEAD;
                        hidx       <= 2'd0;
                        didx       <= 16'd0;
                        a_llp      <= llp;
                        a_off      <= k2_off[14:0];
                        a_off_full <= k2_off;
                        tx_active  <= 1'b1;
                        lat        <= 32'd0;
                        lat_done   <= 1'b0;
                    end
                end else begin
                    // Any other packet breaks the kind-2 / status pairing.
                    pending_k2 <= 1'b0;
                end
            end

            // ---- the answer in flight ----
            if (astate != A_IDLE && !lat_done) lat <= lat + 32'd1;

            // Each byte is a two-phase handshake: raise `tx_valid` with the
            // byte, then advance only when `tx_ready` is high too — the
            // producer pattern `ip/bus/seph_mcu` uses, so the UART never sees a
            // byte change under it and the responder never races the ready.
            case (astate)
                A_HEAD: begin
                    if (!tx_valid) begin
                        tx_data  <= head_byte;
                        tx_valid <= 1'b1;
                    end else if (tx_ready) begin
                        if (!lat_done) lat_done <= 1'b1;
                        tx_valid <= 1'b0;
                        if (hidx == 2'd2) begin
                            if (a_llp == 16'd0) begin
                                astate <= A_DONE;
                            end else begin
                                astate  <= A_DATA;
                                rd_addr <= a_off;   // first image byte
                                prep    <= 1'b1;
                            end
                        end else begin
                            hidx <= hidx + 2'd1;
                        end
                    end
                end
                A_DATA: begin
                    if (prep) begin
                        // Let the registered image read settle for one cycle.
                        prep <= 1'b0;
                    end else if (!tx_valid) begin
                        tx_data  <= rd_data;
                        tx_valid <= 1'b1;
                    end else if (tx_ready) begin
                        if (!lat_done) lat_done <= 1'b1;
                        tx_valid <= 1'b0;
                        if (didx == a_llp - 16'd1) begin
                            astate <= A_DONE;
                        end else begin
                            didx    <= didx + 16'd1;
                            rd_addr <= a_off + didx[14:0] + 15'd1;
                            prep    <= 1'b1;
                        end
                    end
                end
                A_DONE: begin
                    tx_valid     <= 1'b0;
                    tx_active    <= 1'b0;
                    astate       <= A_IDLE;
                    answered_stb <= 1'b1;
                    notice       <= {a_off_full, a_llp, lat};
                    notice_stb   <= 1'b1;
                end
                default: ;   // A_IDLE
            endcase
        end
    end
endmodule
