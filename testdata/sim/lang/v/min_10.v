`timescale 1ns/1ps
module tb;
  initial begin
    fork
      #2 $display("%0t b", $time);
      #1 $display("%0t a", $time);
    join
    $display("%0t joined", $time);
    $finish;
  end
endmodule
