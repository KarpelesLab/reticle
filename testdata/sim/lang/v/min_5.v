`timescale 1ns/1ps
module tb;
  reg [7:0] m [0:1][0:1];
  initial begin
    m[1][0] = 8'h5a;
    $display("%h", m[1][0]);
    $finish;
  end
endmodule
