`timescale 1ns/1ps
module tb;
  int a = 5;
  initial begin
    assert (a == 6) $display("M1 pass action"); else $display("M1 else action");
    $finish;
  end
endmodule
