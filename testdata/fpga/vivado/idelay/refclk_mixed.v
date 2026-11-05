module refclk_mixed(input clk, input c0, input c1, input c2, input c3, output reg r, input p0, input p1);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  IDELAYE2 #(.REFCLK_FREQUENCY(190.0)) u0(.IDATAIN(p0), .DATAOUT(d0));
  wire d1;
  IDELAYE2 #(.REFCLK_FREQUENCY(210.0)) u1(.IDATAIN(p1), .DATAOUT(d1));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= d0 ^ d1 ^ rdy;
endmodule
