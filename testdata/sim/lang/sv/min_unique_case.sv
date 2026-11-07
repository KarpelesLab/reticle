`timescale 1ns/1ps
module tb;
  logic [1:0] v = 2'd3;
  logic [3:0] o = 0;
  initial begin
    unique case (v)
      2'd0: o = 4'h1;
      2'd1: o = 4'h2;
    endcase
    $display("M1 o=%h", o);
    $finish;
  end
endmodule
