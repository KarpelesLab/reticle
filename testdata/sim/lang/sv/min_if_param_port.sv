`timescale 1ns/1ps
interface p_if #(parameter int W = 8);
  logic [W-1:0] q;
endinterface
module drv (p_if s, input logic [15:0] d);
  assign s.q = d;
endmodule
module rd (p_if s, output logic [15:0] o);
  assign o = s.q;
endmodule
module tb;
  logic [15:0] d, o4, o8;
  p_if #(.W(4)) a ();
  p_if b ();
  drv da (.s(a), .d(d));
  drv db (.s(b), .d(d));
  rd ra (.s(a), .o(o4));
  rd rb (.s(b), .o(o8));
  initial begin
    d = 16'hFFFF;
    #1 $display("M1 o4=%h o8=%h", o4, o8);
    $finish;
  end
endmodule
