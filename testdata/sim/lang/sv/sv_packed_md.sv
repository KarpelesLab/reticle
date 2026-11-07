`timescale 1ns/1ps
module tb;
  logic [3:0][7:0] x;
  logic [1:0][2:0][3:0] y;
  bit [0:3][1:0] z;
  logic [7:0] t;
  int i, j;
  initial begin
    x = 32'hDEADBEEF;
    $display("P1 x=%h x[3]=%h x[0]=%h x[2][7:4]=%h", x, x[3], x[0], x[2][7:4]);
    x[1] = 8'h00;
    $display("P2 x=%h", x);
    x[3][0] = 1'b0;
    $display("P3 x=%h", x);
    $display("P4 x[2:1]=%h", x[2:1]);
    y = '0;
    y[1][2] = 4'hF;
    y[0][0] = 4'h3;
    $display("P5 y=%h bits=%0d", y, $bits(y));
    $display("P6 y[1]=%h y[0]=%h", y[1], y[0]);
    z = 8'b00_01_10_11;
    $display("P7 z[0]=%b z[3]=%b", z[0], z[3]);
    for (i = 0; i < 4; i++) begin
      t = x[i];
      $display("P8 x[%0d]=%h", i, t);
    end
    x = {8'h1, 8'h2, 8'h3, 8'h4};
    $display("P9 x=%h", x);
    x[i-1] = 8'hAA;
    $display("P10 x=%h", x);
    $display("P11 dims=%0d left=%0d right=%0d size2=%0d", $dimensions(x), $left(x), $right(x), $size(x, 2));
    j = 2;
    x[j][j +: 2] = 2'b11;
    $display("P12 x=%h", x);
    $display("P13 z=%b z[1:2]=%b", z, z[1:2]);
    $finish;
  end
endmodule
