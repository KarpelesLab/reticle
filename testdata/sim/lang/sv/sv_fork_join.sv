`timescale 1ns/1ps
module tb;
  int order [$];
  initial begin
    fork
      begin #3 $display("FJ1 a t=%0t", $time); end
      begin #1 $display("FJ1 b t=%0t", $time); end
      #2 $display("FJ1 c t=%0t", $time);
    join
    $display("FJ2 after join t=%0t", $time);
    fork
      #5 $display("FJ3 any slow t=%0t", $time);
      #1 $display("FJ3 any fast t=%0t", $time);
    join_any
    $display("FJ4 after join_any t=%0t", $time);
    fork
      #2 $display("FJ5 none t=%0t", $time);
    join_none
    $display("FJ6 after join_none t=%0t", $time);
    #10;
    $display("FJ7 end t=%0t", $time);
    $finish;
  end
endmodule
