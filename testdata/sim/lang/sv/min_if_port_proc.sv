`timescale 1ns/1ps
interface sig_if;
  logic [7:0] v;
endinterface
module drv (sig_if s);
  initial s.v = 8'h77;
endmodule
module tb;
  sig_if si ();
  drv d (.s(si));
  initial begin
    #1 $display("M1 si.v=%h", si.v);
    $finish;
  end
endmodule
