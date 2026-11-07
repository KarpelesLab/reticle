`timescale 1ns/1ps
module tb;
  function automatic int fact(int n);
    if (n < 2) return 1;
    return n * fact(n - 1);
  endfunction
  initial begin
    $display("M1 fact(6)=%0d", fact(6));
    $finish;
  end
endmodule
