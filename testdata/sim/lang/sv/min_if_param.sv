`timescale 1ns/1ps
interface p_if #(parameter int W = 8);
  logic [W-1:0] q;
endinterface
module tb;
  p_if #(.W(4)) a ();
  p_if b ();
  initial begin
    $display("M1 bits a.q=%0d b.q=%0d", $bits(a.q), $bits(b.q));
    $finish;
  end
endmodule
