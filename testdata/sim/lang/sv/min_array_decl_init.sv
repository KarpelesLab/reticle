`timescale 1ns/1ps
module tb;
  logic [7:0] m [2] = '{8'h11, 8'h22};
  initial begin
    $display("M1 m=%h %h", m[0], m[1]);
    $finish;
  end
endmodule
