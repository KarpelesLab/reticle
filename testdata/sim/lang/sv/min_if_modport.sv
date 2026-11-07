`timescale 1ns/1ps
interface sig_if;
  logic [7:0] v;
  logic [7:0] w;
  modport m (output v, input w);
endinterface
module drv (sig_if.m s);
  assign s.v = s.w + 8'd1;
endmodule
module tb;
  sig_if si ();
  drv d (.s(si));
  initial begin
    si.w = 8'd9;
    #1 $display("M1 si.v=%0d", si.v);
    $finish;
  end
endmodule
