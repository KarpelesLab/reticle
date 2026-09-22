// tmds_encoder — one channel of the DVI 1.0 TMDS 8b/10b encoder.
//
// What it does
//   Exactly the algorithm of the DVI 1.0 specification, section 3.2.2.
//   While `de` is high the eight data bits are first transition
//   minimised — XORed or XNORed bit by bit into q_m, whichever gives
//   fewer transitions, with q_m[8] saying which — and then DC balanced:
//   a running disparity counts the ones sent minus the zeros, and each
//   symbol's low eight bits are inverted, with q_out[9] saying so,
//   whenever that moves the count back towards zero. While `de` is low
//   the two control bits select one of the four control-period symbols,
//   which are chosen for their many transitions, and the disparity is
//   reset to zero.
//
//   `q` is registered and changes only on an edge where `en` is high,
//   so a design running at a multiple of the pixel rate clocks the
//   encoder with a pixel enable. Bit 0 of `q` is the one transmitted
//   first.
//
// What it does not do
//   No HDMI data islands or guard bands (TERC4), no video preamble, and
//   so no audio or InfoFrames: this is DVI, which HDMI sinks accept.
//   The running disparity is held in six bits; the testbench walks
//   every state the algorithm can reach from zero and finds it never
//   leaves -8..+8, so six is two more than it needs.
module tmds_encoder (
    input  wire       clk,
    input  wire       rst_n,
    input  wire       en,
    input  wire       de,
    input  wire [1:0] c,
    input  wire [7:0] d,
    output wire [9:0] q
);
    // Stage one: transition minimisation.
    wire [3:0] n1d = d[0] + d[1] + d[2] + d[3] + d[4] + d[5] + d[6] + d[7];
    wire       use_xnor = (n1d > 4'd4) | ((n1d == 4'd4) & ~d[0]);

    wire [8:0] q_m;
    assign q_m[0] = d[0];
    assign q_m[1] = use_xnor ? ~(q_m[0] ^ d[1]) : (q_m[0] ^ d[1]);
    assign q_m[2] = use_xnor ? ~(q_m[1] ^ d[2]) : (q_m[1] ^ d[2]);
    assign q_m[3] = use_xnor ? ~(q_m[2] ^ d[3]) : (q_m[2] ^ d[3]);
    assign q_m[4] = use_xnor ? ~(q_m[3] ^ d[4]) : (q_m[3] ^ d[4]);
    assign q_m[5] = use_xnor ? ~(q_m[4] ^ d[5]) : (q_m[4] ^ d[5]);
    assign q_m[6] = use_xnor ? ~(q_m[5] ^ d[6]) : (q_m[5] ^ d[6]);
    assign q_m[7] = use_xnor ? ~(q_m[6] ^ d[7]) : (q_m[6] ^ d[7]);
    assign q_m[8] = ~use_xnor;

    // Stage two: DC balance. `cnt` is the running disparity in two's
    // complement; `twice_n1` is twice the ones in q_m[7:0], so the
    // disparity of those eight bits, N1 - N0, is twice_n1 - 8.
    wire [3:0] n1q = q_m[0] + q_m[1] + q_m[2] + q_m[3] + q_m[4] + q_m[5] + q_m[6] + q_m[7];
    wire [5:0] twice_n1 = {1'b0, n1q, 1'b0};
    reg  [5:0] cnt;
    wire       cnt_zero = (cnt == 6'd0);
    wire       cnt_neg  = cnt[5];
    wire       balanced = (n1q == 4'd4);
    wire       more_ones  = (n1q > 4'd4);
    wire       more_zeros = (n1q < 4'd4);

    reg [9:0] q_next;
    reg [5:0] cnt_next;
    always @(*) begin
        if (!de) begin
            case (c)
                2'b00:   q_next = 10'b1101010100;
                2'b01:   q_next = 10'b0010101011;
                2'b10:   q_next = 10'b0101010100;
                default: q_next = 10'b1010101011;
            endcase
            cnt_next = 6'd0;
        end else if (cnt_zero | balanced) begin
            q_next = {~q_m[8], q_m[8], q_m[8] ? q_m[7:0] : ~q_m[7:0]};
            if (q_m[8]) cnt_next = cnt + twice_n1 - 6'd8;
            else        cnt_next = cnt + 6'd8 - twice_n1;
        end else if ((~cnt_neg & more_ones) | (cnt_neg & more_zeros)) begin
            q_next   = {1'b1, q_m[8], ~q_m[7:0]};
            cnt_next = cnt + {4'd0, q_m[8], 1'b0} + 6'd8 - twice_n1;
        end else begin
            q_next   = {1'b0, q_m[8], q_m[7:0]};
            cnt_next = cnt - {4'd0, ~q_m[8], 1'b0} + twice_n1 - 6'd8;
        end
    end

    reg [9:0] q_q;
    assign q = q_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            q_q <= 10'd0;
            cnt <= 6'd0;
        end else if (en) begin
            q_q <= q_next;
            cnt <= cnt_next;
        end
    end
endmodule
