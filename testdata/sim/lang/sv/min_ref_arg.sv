`timescale 1ns/1ps
module tb;
  int x;
  task automatic tref(ref int v, input int by);
    v += by;
  endtask
  initial begin
    x = 5;
    tref(x, 10);
    $display("M1 x=%0d", x);
    $finish;
  end
endmodule
