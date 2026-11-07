`timescale 1ns/1ps
module tb;
  int a [4];
  int m [2][3];
  logic [3:0][1:0] p;
  logic [7:0] r [3:1];
  int sum;
  initial begin
    foreach (a[i]) a[i] = i * i;
    foreach (a[i]) $display("F1 a[%0d]=%0d", i, a[i]);
    foreach (m[i, j]) m[i][j] = i * 10 + j;
    foreach (m[i, j]) $display("F2 m[%0d][%0d]=%0d", i, j, m[i][j]);
    sum = 0;
    foreach (m[i]) sum += m[i][0];
    $display("F3 sum col0=%0d", sum);
    p = 8'b11_10_01_00;
    foreach (p[k]) $display("F4 p[%0d]=%b", k, p[k]);
    foreach (r[k]) r[k] = k;
    foreach (r[k]) $display("F5 r[%0d]=%0d", k, r[k]);
    foreach (m[i, j]) if (j == 1) $display("F6 pick m[%0d][1]=%0d", i, m[i][j]);
    $finish;
  end
endmodule
