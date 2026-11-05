module refclk_190(input clk, input c0, input c1, input c2, input c3, output reg r, input p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  IDELAYE2 #(.REFCLK_FREQUENCY(190.0)) u0(.IDATAIN(p0), .DATAOUT(d0));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= d0 ^ rdy;
endmodule
