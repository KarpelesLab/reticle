// usb_fs_tx — the transmit half of a USB full-speed PHY.
//
// What it does
//   Sends one packet at 12 Mbit/s from a 48 MHz clock, a bit every four
//   cycles: the SYNC field, the PID with its check nibble, for a data
//   packet the payload and its CRC16, and the end of packet — two bit
//   times of SE0 and one of J — before letting go of the pair. Bits go
//   out least significant first, NRZI encoded (a zero is a change of
//   state, a one is none), with a zero stuffed after every six ones in a
//   row, counted from the SYNC field on and applied before the EOP too.
//
//   `start` takes `pid`, `with_data` and `len`, and the payload is
//   fetched a byte at a time: `index` says which byte is wanted and
//   `byte_in` must hold it in the same cycle. The CRC16 is computed as
//   the payload goes out — polynomial 0x8005, reflected, seeded with all
//   ones and sent complemented, low byte first.
//
// What it does not do
//   Full speed only, and eight bytes of payload at most, which is what a
//   control endpoint with a maximum packet size of eight needs. No
//   pre-amble for low-speed devices behind a hub.
module usb_fs_tx (
    input  wire       clk,
    input  wire       rst_n,
    input  wire       start,
    input  wire [3:0] pid,
    input  wire       with_data,
    input  wire [3:0] len,
    output wire [3:0] index,
    input  wire [7:0] byte_in,
    output wire       busy,
    output wire       dp,
    output wire       dn,
    output wire       oe
);
    localparam [2:0] S_IDLE = 3'd0;
    localparam [2:0] S_SYNC = 3'd1;
    localparam [2:0] S_PID  = 3'd2;
    localparam [2:0] S_DATA = 3'd3;
    localparam [2:0] S_CRC0 = 3'd4;
    localparam [2:0] S_CRC1 = 3'd5;
    localparam [2:0] S_EOP  = 3'd6;

    reg [2:0]  state;
    reg [1:0]  div;
    reg [7:0]  shift;
    reg [2:0]  bitcnt;
    reg [2:0]  ones;
    reg        level;     // 1 = J
    reg        se0;
    reg        oe_q;
    reg [3:0]  idx;
    reg [3:0]  len_q;
    reg        data_q;
    reg [15:0] crc;
    reg [1:0]  eop_cnt;

    wire tick = (div == 2'd3);

    // The state after the byte in `shift` has gone out.
    wire [2:0] after_pid  = !data_q ? S_EOP : (len_q == 4'd0) ? S_CRC0 : S_DATA;
    wire [2:0] after_data = (idx == len_q) ? S_CRC0 : S_DATA;

    assign index = idx;
    assign busy  = (state != S_IDLE);
    assign oe    = oe_q;
    assign dp    = oe_q & ~se0 & level;
    assign dn    = oe_q & ~se0 & ~level;

    function [15:0] crc16_bit;
        input [15:0] c;
        input        b;
        begin
            crc16_bit = (c[0] ^ b) ? ((c >> 1) ^ 16'hA001) : (c >> 1);
        end
    endfunction

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state   <= S_IDLE;
            div     <= 2'd0;
            shift   <= 8'd0;
            bitcnt  <= 3'd0;
            ones    <= 3'd0;
            level   <= 1'b1;
            se0     <= 1'b0;
            oe_q    <= 1'b0;
            idx     <= 4'd0;
            len_q   <= 4'd0;
            data_q  <= 1'b0;
            crc     <= 16'hFFFF;
            eop_cnt <= 2'd0;
        end else begin
            div <= div + 2'd1;
            if (state == S_IDLE) begin
                div <= 2'd0;
                if (start) begin
                    state  <= S_SYNC;
                    shift  <= 8'h80;
                    bitcnt <= 3'd0;
                    ones   <= 3'd0;
                    level  <= 1'b1;
                    idx    <= 4'd0;
                    len_q  <= len;
                    data_q <= with_data;
                    crc    <= 16'hFFFF;
                    // Hold the pair at J for the first bit time's worth
                    // of setup; the SYNC field's first zero follows.
                    div    <= 2'd3;
                end
            end else if (state == S_EOP) begin
                if (tick && ones == 3'd6) begin
                    // Six ones ended the packet: the stuffed zero is
                    // still owed before the EOP.
                    level <= ~level;
                    ones  <= 3'd0;
                end else if (tick) begin
                    // Two bit times of SE0, one of J, then release.
                    eop_cnt <= eop_cnt + 2'd1;
                    se0     <= (eop_cnt != 2'd2);
                    level   <= 1'b1;
                    oe_q    <= 1'b1;
                    if (eop_cnt == 2'd3) begin
                        state   <= S_IDLE;
                        oe_q    <= 1'b0;
                        se0     <= 1'b0;
                        eop_cnt <= 2'd0;
                    end
                end
            end else if (tick) begin
                oe_q <= 1'b1;
                if (ones == 3'd6) begin
                    // A stuffed zero: a change of state, no data bit.
                    level <= ~level;
                    ones  <= 3'd0;
                end else begin
                    if (!shift[0]) level <= ~level;
                    ones   <= shift[0] ? ones + 3'd1 : 3'd0;
                    if (state == S_DATA) crc <= crc16_bit(crc, shift[0]);
                    shift  <= shift >> 1;
                    bitcnt <= bitcnt + 3'd1;
                    if (bitcnt == 3'd7) begin
                        case (state)
                            S_SYNC: begin
                                state <= S_PID;
                                shift <= {~pid, pid};
                            end
                            S_PID: begin
                                state <= after_pid;
                                shift <= byte_in;
                                if (after_pid == S_DATA) idx <= 4'd1;
                                if (after_pid == S_CRC0) shift <= ~crc[7:0];
                            end
                            S_DATA: begin
                                state <= after_data;
                                if (after_data == S_DATA) begin
                                    shift <= byte_in;
                                    idx   <= idx + 4'd1;
                                end else begin
                                    shift <= ~crc16_bit(crc, shift[0]);
                                end
                            end
                            S_CRC0: begin
                                state <= S_CRC1;
                                shift <= ~crc[15:8];
                            end
                            default: begin
                                // S_CRC1 done: the end of the packet.
                                state <= S_EOP;
                            end
                        endcase
                    end
                end
            end
        end
    end
endmodule
