`timescale 1ns/1ps
module tb;
  function real half(input real r);
    half = r / 2.0;
  endfunction
  real x;
  initial begin
    x = half(3.0);
    $display("a %f", x);
    $display("b %f", half(7.0));
    $finish;
  end
endmodule
