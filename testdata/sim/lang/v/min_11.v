`timescale 1ns/1ps
module tb;
  real r;
  initial begin
    r = 0.5;
    $display("%b %b", r && 1, !r);
    if (r) $display("if real true");
    $finish;
  end
endmodule
