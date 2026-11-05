module idelay_more(input clk, input c0, input c1, input c2, input c3, output reg r, input p0, input p1, input p2, input p3);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));
  wire d1;
  IDELAYE2 #(.IDELAY_VALUE(7)) u1(.IDATAIN(p1), .DATAOUT(d1));
  wire d2;
  IDELAYE2 #(.IS_DATAIN_INVERTED(1'b1)) u2(.IDATAIN(p2), .DATAOUT(d2));
  wire d3;
  IDELAYE2 u3(.IDATAIN(p3), .DATAOUT(d3));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= d0 ^ d1 ^ d2 ^ rdy;
endmodule
