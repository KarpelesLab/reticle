`timescale 1ns/1ps
module tb;
  parameter G = 2.5;
  parameter real H = 0.25;
  localparam real L = H * 2;
  real x;
  initial begin
    x = G; $display("a %f", x);
    x = H; $display("b %f", x);
    x = L; $display("c %f", x);
    x = 2.0 ** 0.5; $display("d %f", x);
    x = 1.0 / 4.0; $display("e %f", x);
    x = H + 0.0; $display("f %f", x);
    $display("g %f", G + x);
    $finish;
  end
endmodule
