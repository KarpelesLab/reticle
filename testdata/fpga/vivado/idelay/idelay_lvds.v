module idelay_lvds(input clk, input c0, input c1, input c2, input c3, output reg r, input p0, input p0n, input p1, input p1n);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire b0; IBUFDS i0(.I(p0), .IB(p0n), .O(b0));
  wire d0;
  IDELAYE2 u0(.IDATAIN(b0), .DATAOUT(d0));
  wire b1; IBUFDS i1(.I(p1), .IB(p1n), .O(b1));
  wire d1;
  IDELAYE2 #(.IDELAY_VALUE(6)) u1(.IDATAIN(b1), .DATAOUT(d1));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= d0 ^ d1 ^ rdy;
endmodule
