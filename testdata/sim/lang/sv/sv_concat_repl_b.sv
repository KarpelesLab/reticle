`timescale 1ns/1ps
module tb;
  logic [3:0] a = 4'hA, b = 4'h5;
  logic [15:0] w;
  logic [7:0] x, y;
  int n;
  initial begin
    w = {a, b, a, b};
    $display("CR1 w=%h", w);
    w = {4{a}};
    $display("CR2 w=%h", w);
    w = {2{a, b}};
    $display("CR3 w=%h", w);
    {x, y} = 16'hCAFE;
    $display("CR4 x=%h y=%h", x, y);
    w = {a, {3{1'b1}}, 1'b0};
    $display("CR5 w=%b", w);
    {x[3:0], y[7:4]} = 8'h96;
    $display("CR6 x=%h y=%h", x, y);
    w = {8'(a), 8'(b)};
    $display("CR8 w=%h", w);
    $display("CR9 bits of {a,b}=%0d", $bits({a, b}));
    w = {a, b} + 1;
    $display("CR10 w=%h", w);
    $finish;
  end
endmodule
