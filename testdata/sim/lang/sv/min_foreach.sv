`timescale 1ns/1ps
module tb;
  int a [3];
  initial begin
    foreach (a[i]) a[i] = i * 2;
    $display("M1 a=%0d %0d %0d", a[0], a[1], a[2]);
    $finish;
  end
endmodule
