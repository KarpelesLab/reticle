module dcireset_rst_tied(input clk, input c0, input c1, input c2, input c3, output reg r);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  reg s; always @(posedge gclk) s <= c0 ^ c2;
  wire locked; DCIRESET u0(.RST(1'b0), .LOCKED(locked));
  always @(posedge gclk) r <= locked ^ c1;
endmodule
