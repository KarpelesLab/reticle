`timescale 1ns/1ps
module tb;
  int count = 0;
  logic clk = 0;
  always #5 clk = ~clk;
  always @(posedge clk) count++;
  final $display("FN1 final count=%0d t=%0t", count, $time);
  final begin
    $display("FN2 second final block");
  end
  initial begin
    #42;
    $display("FN0 before finish count=%0d", count);
    $finish;
  end
endmodule
