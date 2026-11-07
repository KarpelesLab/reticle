`timescale 1ns/1ps
module tb;
  int a [4];
  logic [7:0] b [0:3];
  int c [2][2];
  logic [3:0][7:0] p;
  int i;
  initial begin
    a = '{1, 2, 3, 4};
    $display("AP1 a=%0d %0d %0d %0d", a[0], a[1], a[2], a[3]);
    a = '{default: 9};
    $display("AP2 a=%0d %0d %0d %0d", a[0], a[1], a[2], a[3]);
    a = '{0: 5, 2: 7, default: -1};
    $display("AP3 a=%0d %0d %0d %0d", a[0], a[1], a[2], a[3]);
    b = '{8'hA0, 8'hB0, 8'hC0, 8'hD0};
    $display("AP4 b=%h %h %h %h", b[0], b[1], b[2], b[3]);
    c = '{'{1, 2}, '{3, 4}};
    $display("AP5 c=%0d %0d %0d %0d", c[0][0], c[0][1], c[1][0], c[1][1]);
    c = '{default: '{default: 6}};
    $display("AP6 c=%0d %0d %0d %0d", c[0][0], c[0][1], c[1][0], c[1][1]);
    p = '{8'h11, 8'h22, 8'h33, 8'h44};
    $display("AP7 p=%h", p);
    p = '{default: 8'h5A};
    $display("AP8 p=%h", p);
    a = '{4{3}};
    $display("AP9 a=%0d %0d %0d %0d", a[0], a[1], a[2], a[3]);
    $display("AP10 eq %b", a == '{3, 3, 3, 3});
    $display("AP11 %p", a);
    b = '{default: 'x};
    $display("AP12 b0=%b", b[0]);
    $finish;
  end
endmodule
