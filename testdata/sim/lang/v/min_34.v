`timescale 1ns/1ps
module tb;
  reg signed [7:0] s;
  reg [7:0] u;
  initial begin
    s = 8'b1000000z; u = 8'b1000000z;
    $display("[%d] [%d]", s, u);
    s = 8'b0000000z; u = 8'b0000000z;
    $display("[%d] [%d]", s, u);
    s = 8'bzzzzzzzz;
    $display("[%d]", s);
    $finish;
  end
endmodule
