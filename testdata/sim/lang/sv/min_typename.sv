`timescale 1ns/1ps
module tb;
  int i;
  logic [3:0] l;
  initial begin
    $display("M1 %s | %s", $typename(i), $typename(l));
    $finish;
  end
endmodule
