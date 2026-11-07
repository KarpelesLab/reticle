`timescale 1ns/1ps
module tb;
  initial begin
    $display("%m");
    begin : blk
      $display("%m");
    end
    $display("plusargs %0d", $test$plusargs("nonexistent"));
    $finish;
  end
endmodule
