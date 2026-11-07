`timescale 1ns/1ps
module tb;
  logic [7:0] a = 8'b1100_1010, r;
  initial begin
    r = {<<{a}};
    $display("M1 r=%b", r);
    $finish;
  end
endmodule
