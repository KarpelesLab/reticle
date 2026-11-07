`timescale 1ns/1ps
module tb;
  int i, j;
  initial begin
    i = 5;
    j = i++;
    $display("M1 j=i++ j=%0d i=%0d", j, i);
    j = i--;
    $display("M2 j=i-- j=%0d i=%0d", j, i);
    $finish;
  end
endmodule
