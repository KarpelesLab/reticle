`timescale 1ns/1ps
module tb;
  integer i;
  initial begin
    i = 0; repeat (2'bx1) i = i + 1; $display("x1 %0d", i);
    i = 0; repeat (3'bx11) i = i + 1; $display("x11 %0d", i);
    i = 0; repeat (1'bx) i = i + 1; $display("x %0d", i);
    $finish;
  end
endmodule
