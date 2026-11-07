`timescale 1ns/1ps
module tb;
  logic [3:0] k;
  always_comb k = 4'd7;
  initial $display("M1 t=0 k=%b", k);
  initial #1 $finish;
endmodule
