`timescale 1ns/1ps
module sub #(parameter real SCALE = 1.5) ();
  initial $display("M1 SCALE=%f", SCALE);
endmodule
module tb;
  parameter real P = 0.25;
  localparam real Q = 2.75;
  sub s0 ();
  sub #(.SCALE(0.25)) s1 ();
  initial begin
    #1 $display("M2 P=%f Q=%f", P, Q);
    $finish;
  end
endmodule
