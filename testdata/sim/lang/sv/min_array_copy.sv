`timescale 1ns/1ps
module tb;
  logic [7:0] m [2];
  logic [7:0] n [2];
  initial begin
    m[0] = 8'h11; m[1] = 8'h22;
    n = m;
    $display("M1 n=%h %h", n[0], n[1]);
    $finish;
  end
endmodule
