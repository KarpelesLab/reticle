module oserdes_tfb_pipelined(input clk, input clkd, input c0, input c1, input c2, input c3, output reg r, output p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire gdiv; BUFG bd(.I(clkd), .O(gdiv));
  reg [15:0] sh; always @(posedge gdiv) sh <= {sh[14:0], c3};
  wire tfb_0;
  OSERDESE2 u0(.OQ(p0), .TFB(tfb_0), .CLK(gclk), .CLKDIV(gdiv), .OCE(c0), .RST(c1), .D1(sh[0]), .D2(sh[1]), .D3(sh[2]), .D4(sh[3]), .D5(sh[4]), .D6(sh[5]), .D7(sh[6]), .D8(sh[7]));
  reg r0; always @(posedge gdiv) r0 <= tfb_0;
  always @(posedge gdiv) r <= r0;
endmodule
