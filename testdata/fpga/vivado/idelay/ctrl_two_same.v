module ctrl_two_same(input clk, input c0, input c1, input c2, input c3, output reg r, input p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));
  wire rdy0, rdy1;
  IDELAYCTRL ctl0(.REFCLK(gclk), .RST(c3), .RDY(rdy0));
  IDELAYCTRL ctl1(.REFCLK(gclk), .RST(c2), .RDY(rdy1));
  wire rdy = rdy0 ^ rdy1;
  always @(posedge gclk) r <= d0 ^ rdy;
endmodule
