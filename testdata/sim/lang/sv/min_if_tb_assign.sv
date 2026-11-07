`timescale 1ns/1ps
interface sig_if;
  logic [7:0] v;
endinterface
module tb;
  sig_if si ();
  assign si.v = 8'hC3;
  initial begin
    #1 $display("M1 si.v=%h", si.v);
    $finish;
  end
endmodule
