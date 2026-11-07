`timescale 1ns/1ps
// sv_packed_md without writes to a select of a packed element or to a variable element.
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
    $display("P4 x[2:1]=%h", x[2:1]);
    y = '0;
    y[1] = 12'hF00;
    y[0] = 12'h003;
    $display("P5 y=%h bits=%0d", y, $bits(y));
    $display("P6 y[1]=%h y[0]=%h y[1][2]=%h y[0][0]=%h", y[1], y[0], y[1][2], y[0][0]);
    z = 8'b00_01_10_11;
    $display("P7 z[0]=%b z[3]=%b", z[0], z[3]);
    for (i = 0; i < 4; i++) begin
      t = x[i];
      $display("P8 x[%0d]=%h", i, t);
    end
    x = {8'h1, 8'h2, 8'h3, 8'h4};
    $display("P9 x=%h", x);
    $display("P11 dims=%0d left=%0d right=%0d size2=%0d", $dimensions(x), $left(x), $right(x), $size(x, 2));
    j = 2;
    $display("P12 x[j][j+:2]=%b x[j+1]=%h", x[j][j +: 2], x[j+1]);
    $display("P13 z=%b z[1:2]=%b", z, z[1:2]);
    x[3:2] = 16'hCAFE;
    $display("P14 x=%h", x);
    $finish;
  end
endmodule
