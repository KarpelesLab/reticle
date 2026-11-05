module datain_with_pad(input clk, input c0, input c1, input c2, input c3, output reg r, input p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  reg s; always @(posedge gclk) s <= c0 ^ c2;
  wire d0, d1;
  IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));
  IDELAYE2 #(.DELAY_SRC("DATAIN")) u1(.DATAIN(s), .DATAOUT(d1));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= d0 ^ d1 ^ rdy;
endmodule
