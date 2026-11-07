`timescale 1ns/1ps
module tb;
  bit b;
  int i;
  byte y;
  initial begin
    $display("M1 initial b=%b i=%0d y=%0d", b, i, y);
    b = 1'bx;
    i = 'x;
    y = 8'b1x0z_1x0z;
    $display("M2 after x: b=%b i=%0d y=%b", b, i, y);
    $display("M3 bit'(1'bx)=%b int'(4'bx1x0)=%0d", bit'(1'bx), int'(4'bx1x0));
    $finish;
  end
endmodule
