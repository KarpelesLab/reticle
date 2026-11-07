`timescale 1ns/1ps
module tb;
  int a [8];
  int b [4];
  int c [0:3];
  logic [7:0] d [0:7];
  logic [7:0] e [0:1];
  int i;
  initial begin
    for (i = 0; i < 8; i++) a[i] = i * 3;
    b = a[2:5];
    $display("SL1 b=%0d %0d %0d %0d", b[0], b[1], b[2], b[3]);
    c = a[4:7];
    $display("SL2 c=%0d %0d %0d %0d", c[0], c[1], c[2], c[3]);
    a[0:3] = c;
    $display("SL3 a=%0d %0d %0d %0d %0d", a[0], a[1], a[2], a[3], a[4]);
    for (i = 0; i < 8; i++) d[i] = 8'h10 + i;
    e = d[6:7];
    $display("SL4 e=%h %h", e[0], e[1]);
    d[0:1] = e;
    $display("SL5 d0=%h d1=%h", d[0], d[1]);
    $display("SL6 eq %b", a[0:3] == c);
    $finish;
  end
endmodule
