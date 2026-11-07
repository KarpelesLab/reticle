`timescale 1ns/1ps
module tb;
  real r;
  initial begin
    r = real'(7);
    $display("M1 real'(7)/2=%f r/2=%f", real'(7) / 2, r / 2);
    $finish;
  end
endmodule
