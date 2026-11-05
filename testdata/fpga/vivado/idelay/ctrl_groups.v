module ctrl_groups(input clk, input c0, input c1, input c2, input c3, output reg r, input p0, input p1);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  (* IODELAY_GROUP = "g0" *) IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));
  wire d1;
  (* IODELAY_GROUP = "g1" *) IDELAYE2 u1(.IDATAIN(p1), .DATAOUT(d1));
  wire rdy0, rdy1;
  (* IODELAY_GROUP = "g0" *) IDELAYCTRL ctl0(.REFCLK(gclk), .RST(c3), .RDY(rdy0));
  (* IODELAY_GROUP = "g1" *) IDELAYCTRL ctl1(.REFCLK(gclk), .RST(c3), .RDY(rdy1));
  wire rdy = rdy0 ^ rdy1;
  always @(posedge gclk) r <= d0 ^ d1 ^ rdy;
endmodule
