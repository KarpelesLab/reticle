`timescale 1ns/1ps
module tb;
  int m [2][3];
  initial begin
    m[1][2] = 7;
    $display("M1 m[1][2]=%0d", m[1][2]);
    $finish;
  end
endmodule
