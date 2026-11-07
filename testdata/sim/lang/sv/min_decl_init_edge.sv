`timescale 1ns/1ps
module tb;
  logic clk = 0;
  int ne = 0, ed = 0, ch = 0;
  always @(negedge clk) ne++;
  always @(edge clk) ed++;
  always @(clk) ch++;
  initial #1 begin
    $display("M1 ne=%0d ed=%0d ch=%0d", ne, ed, ch);
    $finish;
  end
endmodule
