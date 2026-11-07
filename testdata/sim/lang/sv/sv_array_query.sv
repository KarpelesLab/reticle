`timescale 1ns/1ps
module tb;
  logic [7:0] v;
  logic [0:15] w;
  logic [3:0][7:0] p;
  logic [7:0] u [2:5];
  int q [10];
  logic [5:2][1:0] m [0:2][3:1];
  initial begin
    $display("Q1 v: size=%0d left=%0d right=%0d low=%0d high=%0d inc=%0d dim=%0d bits=%0d", $size(v), $left(v), $right(v), $low(v), $high(v), $increment(v), $dimensions(v), $bits(v));
    $display("Q2 w: size=%0d left=%0d right=%0d low=%0d high=%0d inc=%0d", $size(w), $left(w), $right(w), $low(w), $high(w), $increment(w));
    $display("Q3 p: size=%0d size2=%0d left2=%0d dim=%0d bits=%0d", $size(p), $size(p, 2), $left(p, 2), $dimensions(p), $bits(p));
    $display("Q4 u: size=%0d left=%0d right=%0d low=%0d high=%0d inc=%0d", $size(u), $left(u), $right(u), $low(u), $high(u), $increment(u));
    $display("Q5 u: dim=%0d unpacked_dim=%0d bits=%0d", $dimensions(u), $unpacked_dimensions(u), $bits(u));
    $display("Q6 q: size=%0d high=%0d", $size(q), $high(q));
    $display("Q7 m: dim=%0d updim=%0d bits=%0d", $dimensions(m), $unpacked_dimensions(m), $bits(m));
    $display("Q8 m: size1=%0d size2=%0d size3=%0d size4=%0d", $size(m, 1), $size(m, 2), $size(m, 3), $size(m, 4));
    $display("Q9 m: left2=%0d right2=%0d left3=%0d right3=%0d inc2=%0d", $left(m, 2), $right(m, 2), $left(m, 3), $right(m, 3), $increment(m, 2));
    $display("Q10 int: bits=%0d left=%0d", $bits(int), $left(int));
    $display("Q11 bits(expr)=%0d bits(concat)=%0d", $bits(v + 1'b1), $bits({v, w}));
    $finish;
  end
endmodule
