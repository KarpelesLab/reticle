`timescale 1ns/1ps
module tb;
  logic [3:0][7:0] x;
  int i;
  initial begin
    x = 32'h0;
    i = 2;
    x[i] = 8'hAA;
    $display("M1 x=%h", x);
    $finish;
  end
endmodule
