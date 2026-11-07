`timescale 1ns/1ps
module tb;
  reg clk;
  reg [7:0] a, b, c, d, e;
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin
    a = 1; b = 0; c = 0; d = 0; e = 0;
    #1;
    b = @(posedge clk) a;
    $display("%0t b=%0d (event intra blocking)", $time, b);
    a = 2;
    c <= @(posedge clk) a;
    a = 3;
    $display("%0t c=%0d right after", $time, c);
    @(posedge clk); #1;
    $display("%0t c=%0d (sampled 2)", $time, c);
    a = 4;
    d <= repeat (2) @(posedge clk) a;
    a = 5;
    @(posedge clk); #1 $display("%0t d=%0d after 1 edge", $time, d);
    @(posedge clk); #1 $display("%0t d=%0d after 2 edges", $time, d);
    e = #3 a + 10;
    $display("%0t e=%0d", $time, e);
    e <= #0 99;
    $display("%0t e=%0d before #0 nba", $time, e);
    #0 $display("%0t e=%0d after #0", $time, e);
    #1 $display("%0t e=%0d after #1", $time, e);
    @(negedge clk) $display("%0t negedge", $time);
    @(clk) $display("%0t any edge", $time);
    repeat (3) @(posedge clk);
    $display("%0t after 3 posedges", $time);
    #(1.5) $display("%0t real delay %t", $time, $realtime);
    $finish;
  end
endmodule
