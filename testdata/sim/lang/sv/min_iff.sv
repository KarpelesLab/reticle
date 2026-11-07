`timescale 1ns/1ps
module tb;
  logic clk = 0, en = 0;
  int n = 0;
  always #5 clk = ~clk;
  always @(posedge clk iff en) n++;
  initial begin
    #12 en = 1;
    #20 $display("M1 n=%0d", n);
    $finish;
  end
endmodule
