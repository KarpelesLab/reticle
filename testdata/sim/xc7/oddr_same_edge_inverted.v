`timescale 1ns / 1ps
// The same as oddr_same_edge.v with IS_C_INVERTED: the cycles CE holds
// low now begin on a falling edge, which is what showed that the falling
// edge loads the output only if CE was high at the rising edge before.
module tb;
  reg c = 1'b0, d1 = 0, d2 = 0, ce = 1;
  wire q;
  ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .IS_C_INVERTED(1))  u (.Q(q), .C(c), .CE(ce), .D1(d1), .D2(d2), .R(1'b0), .S(1'b0));
  integer i;
  // rising edges at 10k+5, falling at 10k+10; D1/D2 change at 10k+2 (before
  // the rising edge) and D2 again at 10k+7 (before the falling edge)
  reg [63:0] p1 = 64'hA5C3_96F0_1E2D_3C4B, p2 = 64'h5A3C_69F0_E1D2_C3B4, p3 = 64'hFF00_F0F0_CCCC_AAAA;
  initial begin
    #200;
    for (i = 0; i < 64; i = i + 1) begin
      #2 d1 = p1[i]; d2 = p2[i]; ce = ~(i >= 20 && i < 26 && i[0]) && !(i >= 40 && i < 44);
      #1 $display("%0d a %b %b %b %b", i, q, d1, d2, ce);
      #2 c = 1;
      #1 $display("%0d b %b", i, q);
      #1 d2 = p3[i];
      #1 $display("%0d c %b %b", i, q, d2);
      #2 c = 0;
      #1 $display("%0d d %b", i, q);
    end
    $finish;
  end
endmodule
