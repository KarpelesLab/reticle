module oddr_both(input clk, input c0, input c1, input c2, input c3, output reg r, output p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  ODDR u0(.Q(p0), .C(gclk), .CE(c0), .D1(c2), .D2(c3), .R(c1), .S(c2));
  always @(posedge gclk) r <= 1'b0;
endmodule
