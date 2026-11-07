`timescale 1ns/1ps
module tb;
  function [7:0] add1(input [7:0] x);
    add1 = x + 1;
  endfunction
  function automatic integer fact(input integer n);
    integer q; begin fact = 1; for (q = 2; q <= n; q = q + 1) fact = fact * q; end
  endfunction
  function automatic [31:0] fib(input [31:0] n);
    reg [31:0] p0, p1, t; integer q; begin p0 = 0; p1 = 1; for (q = 0; q < n; q = q + 1) begin t = p0 + p1; p0 = p1; p1 = t; end fib = p0; end
  endfunction
  function signed [7:0] neg(input signed [7:0] x);
    neg = -x;
  endfunction
  function integer clog2(input integer v);
    integer t;
    begin
      clog2 = 0; t = v - 1;
      while (t > 0) begin clog2 = clog2 + 1; t = t >> 1; end
    end
  endfunction
  function real half(input real r);
    half = r / 2.0;
  endfunction
  function [3:0] multi(input [3:0] a, input [3:0] b, input c);
    begin
      if (c) multi = a & b;
      else multi = a | b;
    end
  endfunction
  function [7:0] noarg_style;
    input [7:0] p;
    input [7:0] q;
    noarg_style = p - q;
  endfunction
  function [7:0] early(input [7:0] x);
    integer k;
    begin
      early = 0;
      for (k = 0; k < 8; k = k + 1) if (x[k]) early = k;
    end
  endfunction
  function [15:0] trunc_in(input [3:0] x);
    trunc_in = {x, x, x, x};
  endfunction
  localparam W = clog2(100);
  localparam F5 = fact(5);
  reg [W-1:0] sized;
  integer i;
  reg [7:0] r;
  initial begin
    $display("add1 %0d %0d", add1(8'd5), add1(8'd255));
    $display("add1 ctx %0d", add1(8'd255) + 9'd0);
    $display("fact %0d %0d %0d", fact(0), fact(5), fact(10));
    $display("fib %0d %0d", fib(10), fib(15));
    $display("neg %0d %0d", neg(8'sd5), neg(-8'sd128));
    $display("neg ext %b", neg(8'sd1) + 16'sd0);
    $display("W=%0d F5=%0d", W, F5);
    $display("half %f %f", half(3.0), half(7));
    $display("multi %b %b", multi(4'b1100, 4'b1010, 1), multi(4'b1100, 4'b1010, 0));
    $display("old style %0d", noarg_style(8'd10, 8'd3));
    $display("early %0d %0d", early(8'b0100_1000), early(8'd0));
    $display("trunc_in %h", trunc_in(8'hab));
    r = add1(add1(add1(8'd1))); $display("nest %0d", r);
    for (i = 0; i < 4; i = i + 1) $display("loop fact(%0d)=%0d", i, fact(i));
    $display("func in expr %0d", fact(3) * 2 + add1(8'd0));
    $finish;
  end
endmodule
