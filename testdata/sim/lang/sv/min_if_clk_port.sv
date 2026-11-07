`timescale 1ns/1ps
interface c_if (input logic clk);
  logic [3:0] cnt;
endinterface
module counter (c_if b, output logic [3:0] q);
  logic [3:0] r = 0;
  always @(posedge b.clk) r <= r + 1;
  assign b.cnt = r;
  assign q = r;
endmodule
module tb;
  logic clk = 0;
  logic [3:0] q;
  c_if ci (.clk(clk));
  counter u (.b(ci), .q(q));
  always #5 clk = ~clk;
  initial begin
    #42 $display("M1 q=%0d", q);
    $finish;
  end
endmodule
