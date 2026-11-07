`timescale 1ns/1ps
module tb;
  logic [1:0] v = 2'd3;
  logic [3:0] o = 0;
  initial begin
    unique0 case (v)
      2'd0: o = 4'hC;
      2'd1: o = 4'hD;
    endcase
    $display("M1 o=%h", o);
    $finish;
  end
endmodule
