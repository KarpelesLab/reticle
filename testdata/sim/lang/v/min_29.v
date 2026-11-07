`timescale 1ns/1ps
module tb;
  event ev;
  initial begin
    @(ev) $display("%0t got ev", $time);
  end
  initial begin
    #1 -> ev;
    #1 $finish;
  end
endmodule
