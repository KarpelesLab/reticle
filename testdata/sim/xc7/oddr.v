`timescale 1ns / 1ps
// ODDR in both DDR_CLK_EDGE modes and both SRTYPEs, with each inversion,
// driven by pseudo-random D1, D2, CE, R and S; the outputs printed a
// nanosecond after each clock edge and again before the next.
module tb;
  reg c = 1'b0;
  reg [31:0] lfsr = 32'h0bad_cafe;
  wire d1 = lfsr[0];
  wire d2 = lfsr[1];
  wire ce = lfsr[3] | lfsr[7];
  wire r = lfsr[5] & lfsr[9] & lfsr[13];
  wire s = lfsr[6] & lfsr[10] & lfsr[14] & ~r;
  wire [5:0] q;
  ODDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE"), .SRTYPE("SYNC"), .INIT(1'b0))
    u0 (.Q(q[0]), .C(c), .CE(ce), .D1(d1), .D2(d2), .R(r), .S(s));
  ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .SRTYPE("SYNC"), .INIT(1'b1))
    u1 (.Q(q[1]), .C(c), .CE(ce), .D1(d1), .D2(d2), .R(r), .S(s));
  ODDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE"), .SRTYPE("ASYNC"))
    u2 (.Q(q[2]), .C(c), .CE(ce), .D1(d1), .D2(d2), .R(r), .S(s));
  ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .SRTYPE("ASYNC"), .IS_C_INVERTED(1'b1))
    u3 (.Q(q[3]), .C(c), .CE(ce), .D1(d1), .D2(d2), .R(r), .S(s));
  ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .SRTYPE("SYNC"), .IS_D1_INVERTED(1'b1))
    u4 (.Q(q[4]), .C(c), .CE(ce), .D1(d1), .D2(d2), .R(r), .S(s));
  ODDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE"), .SRTYPE("SYNC"), .IS_D2_INVERTED(1'b1))
    u5 (.Q(q[5]), .C(c), .CE(ce), .D1(d1), .D2(d2), .R(r), .S(s));
  integer i;
  initial begin
    #80.5;
    for (i = 0; i < 200; i = i + 1) begin
      #1 $display("%0d %b", i, q);
      #1 lfsr = {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
      #2 $display("%0d %b", i, q);
      #1 c = ~c;
    end
    $finish;
  end
endmodule
