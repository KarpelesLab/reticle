`timescale 1ns/1ps
module tb;
  reg [7:0] a, b;
  initial begin
    a = 0; b = 0;
    $monitor("%0t mon a=%0d b=%0d", $time, a, b);
    #1 a = 1;
    #1 b = 2;
    #1 a = 3; b = 3;
    #1 a = 4; $display("%0t disp a=%0d", $time, a); $strobe("%0t strobe a=%0d", $time, a); a = 5;
    #1 a <= 6; $display("%0t disp nba a=%0d", $time, a); $strobe("%0t strobe nba a=%0d", $time, a);
    #1 $monitoroff;
    #1 a = 7;
    #1 $monitoron;
    #1 a = 8;
    #1 a = 8;
    #1 $strobeh(a); $strobeb(b);
    #1 a = 9; a = 10;
    #1 $display("%0t end", $time);
    $finish;
  end
endmodule
