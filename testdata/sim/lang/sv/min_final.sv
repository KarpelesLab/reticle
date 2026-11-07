`timescale 1ns/1ps
module tb;
  final $display("M1 final t=%0t", $time);
  initial #3 $finish;
endmodule
