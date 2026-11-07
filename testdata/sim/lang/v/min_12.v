`timescale 1ns/1ps
module sub #(parameter real R = 1.5) ();
  initial #1 $display("R=%f", R);
endmodule
module tb;
  parameter G = 2.5;
  parameter real H = 0.25;
  sub u0 ();
  sub #(2.25) u1 ();
  initial begin
    $display("G=%f H=%f", G, H);
    #2 $finish;
  end
endmodule
