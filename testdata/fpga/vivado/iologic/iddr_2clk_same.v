module iddr_2clk_same(input clk, input c0, input c1, input c2, input c3, output reg r, input p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire q1_0, q2_0;
  IDDR_2CLK u0(.Q1(q1_0), .Q2(q2_0), .C(gclk), .CE(c0), .D(p0), .R(1'b0), .S(1'b0), .CB(gclk));
  always @(posedge gclk) r <= q1_0 ^ q2_0;
endmodule
