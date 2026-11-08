// Recursive functions called with arguments known only at run time: one
// calling itself once (`sum`), one calling itself twice from a `?:`
// (`fib`), and `sum` again past the depth Reticle inlines it to, which xsim
// computes and Reticle stops at with a failure. (`wire [31:0] y = sum(n)`
// works in Reticle too, but crashed xsim 2026.1's simulation engine, so it
// is not here.)
`timescale 1ns/1ps
module tb;
  function automatic integer sum(input integer n);
    if (n <= 0) sum = 0; else sum = n + sum(n - 1);
  endfunction
  function automatic integer fib(input integer n);
    fib = (n < 2) ? n : fib(n - 1) + fib(n - 2);
  endfunction
  integer i;
  initial begin
    i = 10;
    $display("sum(10)=%0d", sum(i));
    for (i = 0; i <= 10; i = i + 1) $display("fib(%0d)=%0d", i, fib(i));
    i = 100;
    $display("sum(100)=%0d", sum(i));
    $finish;
  end
endmodule
