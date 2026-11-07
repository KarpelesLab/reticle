`timescale 1ns/1ps
module tb;
  logic [3:0][7:0] x;
  initial begin
    x = 32'hDEADBEEF;
    x[3][0] = 1'b1;
    $display("M1 x=%h", x);
    $finish;
  end
endmodule
