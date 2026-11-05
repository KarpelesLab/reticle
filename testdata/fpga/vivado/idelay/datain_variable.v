module datain_variable(input clk, input c0, input c1, input c2, input c3, output reg r);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  reg s; always @(posedge gclk) s <= c0 ^ c2;
  wire d; IDELAYE2 #(.DELAY_SRC("DATAIN"),.IDELAY_TYPE("VARIABLE")) u0(.DATAIN(s), .DATAOUT(d), .C(gclk), .CE(c0), .INC(c1));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= d ^ rdy;
endmodule
