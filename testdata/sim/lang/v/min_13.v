`timescale 1ns/1ps
module sub;
  initial $display("sub initial");
endmodule
module tb;
  sub u0 ();
  initial $display("tb initial");
  initial #1 $finish;
endmodule
