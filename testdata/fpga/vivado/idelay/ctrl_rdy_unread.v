module ctrl_rdy_unread(input clk, input c0, input c1, input c2, input c3, output reg r, input p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));
  IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY());
  always @(posedge gclk) r <= d0;
endmodule
