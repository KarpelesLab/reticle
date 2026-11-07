`timescale 1ns/1ps
module tb;
  function automatic integer fact(input integer n);
    if (n <= 1) fact = 1; else fact = n * fact(n - 1);
  endfunction
  integer i;
  initial begin
    i = 5;
    $display("fact %0d", fact(i));
    $finish;
  end
endmodule
