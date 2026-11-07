`timescale 1ns/1ps
module tb;
  function automatic int fib(int n);
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
  endfunction
  localparam int F10 = fib(10);
  initial begin
    $display("M1 F10=%0d runtime fib(7)=%0d", F10, fib(7));
    $finish;
  end
endmodule
