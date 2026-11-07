`timescale 1ns/1ps
module child;
  int val = 3;
endmodule
module tb;
  child c1 ();
  initial begin
    #1 $display("M1 c1.val=%0d", c1.val);
    $finish;
  end
endmodule
