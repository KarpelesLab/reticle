// `repeat (N) @(posedge clk)` as an intra-assignment event: xsim counts
// every change of `clk` in the nonblocking form, posedges only in the
// blocking one. IEEE 1800-2017 §9.4.5 counts posedges in both.
`timescale 1ns/1ps
module tb;
  reg clk;
  reg [7:0] a, d, f, g;
  initial begin clk = 0; forever #5 clk = ~clk; end
  always @(d) $display("%0t d=%0d", $time, d);
  always @(f) $display("%0t f=%0d", $time, f);
  always @(g) $display("%0t g=%0d", $time, g);
  initial begin
    a = 4; d = 0; f = 0; g = 0;
    #16;
    d <= repeat (2) @(posedge clk) a;
    f <= repeat (3) @(posedge clk) a + 1;
    a = 5;
    g = repeat (2) @(posedge clk) a + 2;
    $display("%0t blocking done", $time);
    #50 $finish;
  end
endmodule
