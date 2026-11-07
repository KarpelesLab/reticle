`timescale 1ns/1ps
module tb;
  int d [];
  initial begin
    d = new[2];
    d[1] = 5;
    $display("M1 size=%0d d1=%0d", d.size(), d[1]);
    $finish;
  end
endmodule
