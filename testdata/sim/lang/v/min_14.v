`timescale 1ns/1ps
module tb;
  real r, e;
  initial begin
    r = 0.25;
    e = 0.5;
    $display("a %f", r);
    $display("b %f", 0.25);
    $display("c %f", 2.5E-1);
    $display("d %f", 1.0e20);
    $display("e %f", r + 0.0);
    $display("f %f", 2.0 ** 0.5);
    $display("g %f", 2.0 ** e);
    $display("h %f %f", 1.5, 1.0 / 4.0);
    r = 1.0e20; $display("i %f", r);
    $finish;
  end
endmodule
