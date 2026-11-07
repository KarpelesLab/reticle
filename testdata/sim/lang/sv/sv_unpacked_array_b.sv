`timescale 1ns/1ps
// sv_unpacked_array without multi-dim unpacked arrays, whole-array copy/compare, and %p.
module tb;
  logic [7:0] m [4];
  logic [7:0] n [0:3];
  int a [3:0];
  bit [3:0] c [8];
  int i, j;
  initial begin
    $display("R1 init m0=%h a0=%0d c0=%b", m[0], a[0], c[0]);
    for (i = 0; i < 4; i++) m[i] = i * 8'h11;
    $display("R2 m=%h %h %h %h", m[0], m[1], m[2], m[3]);
    for (i = 0; i < 4; i++) n[i] = m[i];
    $display("R3 n=%h %h %h %h", n[0], n[1], n[2], n[3]);
    for (i = 0; i < 4; i++) a[i] = -i;
    $display("R6 a=%0d %0d %0d %0d", a[3], a[2], a[1], a[0]);
    c[7] = 4'hA;
    $display("R8 c7=%h c7[1]=%b", c[7], c[7][1]);
    m[1][3:0] = 4'h0;
    $display("R9 m1=%h", m[1]);
    i = 5;
    $display("R10 out of range m[i]=%h a[9]=%0d", m[i], a[9]);
    m[i] = 8'h77;
    $display("R11 after oob write m=%h %h %h %h", m[0], m[1], m[2], m[3]);
    $display("R12 m[1][7]=%b", m[1][7]);
    i = 2;
    m[i][i] = 1'b1;
    $display("R13 m[i][i] write m2=%h", m[2]);
    $finish;
  end
endmodule
