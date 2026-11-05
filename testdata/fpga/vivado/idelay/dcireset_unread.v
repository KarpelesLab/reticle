module dcireset_unread(input clk, input c0, input c1, input c2, input c3, output reg r);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  reg s; always @(posedge gclk) s <= c0 ^ c2;
  wire locked; DCIRESET u0(.RST(s), .LOCKED());
  always @(posedge gclk) r <= c1;
endmodule
