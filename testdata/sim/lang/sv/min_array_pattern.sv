`timescale 1ns/1ps
module tb;
  int a [3];
  initial begin
    a = '{1, 2, 3};
    $display("M1 a=%0d %0d %0d", a[0], a[1], a[2]);
    $finish;
  end
endmodule
