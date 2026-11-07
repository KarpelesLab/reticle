`timescale 1ns/1ps
module tb;
  reg clk;
  reg [7:0] c;
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin
    c = 0;
    #1 c <= @(posedge clk) 8'd7;
    $display("%0t continued c=%0d", $time, c);
    #10 $display("%0t c=%0d", $time, c);
    c <= repeat (2) @(posedge clk) 8'd9;
    $display("%0t continued again", $time);
    #30 $display("%0t c=%0d", $time, c);
    $finish;
  end
endmodule
