`timescale 1ns/1ps
module tb;
  int a [4];
  int b [2];
  initial begin
    a[0] = 1; a[1] = 2; a[2] = 3; a[3] = 4;
    b = a[1:2];
    $display("M1 b=%0d %0d", b[0], b[1]);
    $finish;
  end
endmodule
