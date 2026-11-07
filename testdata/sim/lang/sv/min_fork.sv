`timescale 1ns/1ps
module tb;
  initial begin
    fork
      #2 $display("M1 a t=%0t", $time);
      #1 $display("M1 b t=%0t", $time);
    join
    $display("M2 t=%0t", $time);
    $finish;
  end
endmodule
