`timescale 1ns/1ps
module tb;
  reg [3:0] a, b;
  reg [7:0] c;
  reg [15:0] w;
  reg [1:0] p, q;
  reg [2:0] t;
  parameter N = 3;
  initial begin
    a = 4'h5; b = 4'hc;
    $display("cat %h %b", {a, b}, {a, b});
    $display("cat3 %h", {a, 4'h0, b});
    $display("rep %b", {4{2'b10}});
    $display("rep param %b", {N{a}});
    $display("nested %b", {2{a, {2{1'b1}}}});
    $display("rep1 %b", {1{a}});
    w = {a, b}; $display("cat ext %h", w);
    w = {4{a}}; $display("rep ext %h", w);
    c = {a, b, a}; $display("cat trunc %h", c);
    {p, q} = 4'b1101; $display("lhs %b %b", p, q);
    {p, q, t} = 7'b1011010; $display("lhs3 %b %b %b", p, q, t);
    {p, q} = 8'hf6; $display("lhs trunc %b %b", p, q);
    {p, q} = 2'b11; $display("lhs ext %b %b", p, q);
    {c[3:0], c[7:4]} = 8'h12; $display("lhs sel %h", c);
    $display("signed cat %b", {-4'sd1} + 5'd0);
    $display("lit cat %h", {8'd1, 8'd2, 8'd3});
    $display("bit cat %b", {a[0], a[1], a[2], a[3]});
    $display("rep x %b", {3{1'bx}});
    $display("rep expr %b", {(N-1){2'b01}});
    $display("cat in arith %0d", {a, b} + 1);
    $display("cat cmp %b", {a, b} == 8'h5c);
    $display("rep cat %h", {{2{a}}, {2{b}}});
    $display("unsized in cat ok %h", {4'hf, 4'd0});
    $display("string cat %s", {"ab", "cd"});
    $finish;
  end
endmodule
