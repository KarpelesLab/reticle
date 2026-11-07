`timescale 1ns/1ps
module tb;
  reg [7:0] a, b;
  initial begin
    a = 0; b = 0;
    $monitor("%0t mon a=%0d", $time, a);
    #1 a = 1;
    #1 a = 1;
    #1 b = 1;
    #1 $display("%0t nothing", $time);
    #1 $finish;
  end
endmodule
