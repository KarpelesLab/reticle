`timescale 1ns/1ps
module tb;
  reg [3:0] a, v;
  integer nb;
  always @(a) $display("%0t a changed to %0d", $time, a);
  always @(v[2]) nb = nb + 1;
  initial begin
    nb = 0;
    a = 1; v = 0;
    #1 v = 4'b0001; #1 v = 4'b0011; #1 v = 4'b0100;
    #1 $display("nb=%0d (v[2] changed once after t0)", nb);
    a = 2;
    #1 $finish;
  end
endmodule
