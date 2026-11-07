`timescale 1ns/1ps
module tb;
  typedef struct {
    int a;
    logic [7:0] b;
    bit c;
  } rec_t;
  typedef struct {
    rec_t r;
    int arr [3];
  } outer_t;
  rec_t r1, r2;
  outer_t o;
  initial begin
    $display("U1 init a=%0d b=%b c=%b", r1.a, r1.b, r1.c);
    r1 = '{a: 10, b: 8'hAB, c: 1'b1};
    $display("U2 a=%0d b=%h c=%b", r1.a, r1.b, r1.c);
    r2 = r1;
    r2.a = 20;
    $display("U3 r1.a=%0d r2.a=%0d", r1.a, r2.a);
    $display("U4 eq=%b", r1 == r2);
    r2.a = 10;
    $display("U5 eq=%b", r1 == r2);
    r1 = '{5, 8'h12, 0};
    $display("U6 %p", r1);
    o.r = r1;
    o.arr = '{1, 2, 3};
    $display("U7 o.r.b=%h arr=%0d %0d %0d", o.r.b, o.arr[0], o.arr[1], o.arr[2]);
    o = '{r: '{a: -1, b: 8'h00, c: 1}, arr: '{default: 7}};
    $display("U8 %p", o);
    r1 = '{default: 0};
    $display("U9 %p", r1);
    $finish;
  end
endmodule
