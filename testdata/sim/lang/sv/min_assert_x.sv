`timescale 1ns/1ps
module tb;
  initial begin
    assert (1'bx) $display("M1 pass action"); else $display("M1 else action");
    $finish;
  end
endmodule
