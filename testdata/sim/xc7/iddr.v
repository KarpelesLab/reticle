`timescale 1ns / 1ps
// IDDR in each DDR_CLK_EDGE mode and both SRTYPEs, driven by a pseudo-random
// D, CE, R and S; every output printed a nanosecond after each clock edge.
module tb;
  reg c = 1'b0;
  reg [31:0] lfsr = 32'h1234_5678;
  wire d = lfsr[0];
  wire ce = lfsr[3] | lfsr[7];
  wire r = lfsr[5] & lfsr[9] & lfsr[13];
  wire s = lfsr[6] & lfsr[10] & lfsr[14] & ~r;
  wire [11:0] q;
  IDDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE"), .SRTYPE("SYNC"), .INIT_Q1(1'b0), .INIT_Q2(1'b1))
    u0 (.Q1(q[0]), .Q2(q[1]), .C(c), .CE(ce), .D(d), .R(r), .S(s));
  IDDR #(.DDR_CLK_EDGE("SAME_EDGE"), .SRTYPE("SYNC"), .INIT_Q1(1'b1), .INIT_Q2(1'b0))
    u1 (.Q1(q[2]), .Q2(q[3]), .C(c), .CE(ce), .D(d), .R(r), .S(s));
  IDDR #(.DDR_CLK_EDGE("SAME_EDGE_PIPELINED"), .SRTYPE("SYNC"))
    u2 (.Q1(q[4]), .Q2(q[5]), .C(c), .CE(ce), .D(d), .R(r), .S(s));
  IDDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE"), .SRTYPE("ASYNC"))
    u3 (.Q1(q[6]), .Q2(q[7]), .C(c), .CE(ce), .D(d), .R(r), .S(s));
  IDDR #(.DDR_CLK_EDGE("SAME_EDGE"), .SRTYPE("ASYNC"), .IS_C_INVERTED(1'b1))
    u4 (.Q1(q[8]), .Q2(q[9]), .C(c), .CE(ce), .D(d), .R(r), .S(s));
  IDDR #(.DDR_CLK_EDGE("SAME_EDGE_PIPELINED"), .SRTYPE("ASYNC"), .IS_D_INVERTED(1'b1))
    u5 (.Q1(q[10]), .Q2(q[11]), .C(c), .CE(ce), .D(d), .R(r), .S(s));
  integer i;
  initial begin
    #200;
    for (i = 0; i < 160; i = i + 1) begin
      #2 lfsr = {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
      #1 $display("%0d %b", i, q);
      #2 c = ~c;
    end
    $finish;
  end
endmodule
