`timescale 1ns/1ps
module tb;
  typedef logic [3:0] n4_t [2];
  function automatic n4_t pair(logic [3:0] b);
    n4_t t;
    t[0] = b; t[1] = b + 1;
    return t;
  endfunction
  n4_t n;
  initial begin
    n = pair(4'd3);
    $display("M1 n=%0d %0d", n[0], n[1]);
    $finish;
  end
endmodule
