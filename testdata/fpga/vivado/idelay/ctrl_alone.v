module ctrl_alone(input clk, input c0, input c1, input c2, input c3, output reg r);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= rdy ^ c0;
endmodule
