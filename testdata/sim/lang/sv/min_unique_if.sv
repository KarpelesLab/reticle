`timescale 1ns/1ps
module tb;
  logic [1:0] v = 2'd2;
  logic [3:0] o = 0;
  initial begin
    unique if (v == 2'd0) o = 4'h3;
    else if (v == 2'd1) o = 4'h5;
    $display("M1 o=%h", o);
    priority if (v == 2'd0) o = 4'h6;
    else if (v == 2'd1) o = 4'h7;
    $display("M2 o=%h", o);
    v = 2'b11;
    unique if (v[0]) o = 4'h8;
    else if (v[1]) o = 4'h9;
    $display("M3 o=%h", o);
    $finish;
  end
endmodule
