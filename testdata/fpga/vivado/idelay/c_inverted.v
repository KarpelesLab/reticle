module c_inverted(input clk, input c0, input c1, input c2, input c3, output reg r, input p0, input p1, input p2, input p3);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  IDELAYE2 #(.IDELAY_TYPE("VARIABLE"),.IS_C_INVERTED(1'b1)) u0(.IDATAIN(p0), .DATAOUT(d0), .C(gclk), .CE(c0), .INC(c1));
  wire d1;
  IDELAYE2 #(.IDELAY_TYPE("VARIABLE"),.IS_C_INVERTED(1'b1)) u1(.IDATAIN(p1), .DATAOUT(d1), .C(gclk), .CE(c0), .INC(c1));
  wire d2;
  IDELAYE2 #(.IDELAY_TYPE("VARIABLE"),.IS_C_INVERTED(1'b1)) u2(.IDATAIN(p2), .DATAOUT(d2), .C(gclk), .CE(c0), .INC(c1));
  wire d3;
  IDELAYE2 #(.IDELAY_TYPE("VARIABLE"),.IS_C_INVERTED(1'b1)) u3(.IDATAIN(p3), .DATAOUT(d3), .C(gclk), .CE(c0), .INC(c1));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= d0 ^ d1 ^ d2 ^ d3 ^ rdy;
endmodule
