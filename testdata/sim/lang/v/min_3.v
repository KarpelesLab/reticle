`timescale 1ns/1ps
module tb;
  reg signed [3:0] a; reg signed [6:0] b; reg signed [7:0] c; reg signed [9:0] d;
  initial begin
    a = -1; b = -1; c = -1; d = -1;
    $display("[%d] [%d] [%d] [%d]", a, b, c, d);
    $display("[%d]", 4'sd3);
    $finish;
  end
endmodule
