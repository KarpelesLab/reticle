`timescale 1ns/1ps
interface sig_if (input logic clk);
  logic [7:0] v;
endinterface
module rd (sig_if s, output logic [7:0] o, output logic c);
  assign o = s.v;
  assign c = s.clk;
endmodule
module tb;
  logic clk = 1;
  logic [7:0] o;
  logic c;
  sig_if si (.clk(clk));
  assign si.v = 8'hC3;
  rd r (.s(si), .o(o), .c(c));
  initial begin
    #1 $display("M1 o=%h c=%b si.v=%h", o, c, si.v);
    $finish;
  end
endmodule
