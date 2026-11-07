`timescale 1ns/1ps
module tb;
  reg [7:0] a;
  initial begin
    a = 8'd5;
    $display("[%0h] [%0b] [%0o] [%X] [%H]", a, a, a, 8'hab, 8'hcd);
    $strobeh(a);
    $strobeb(a);
    $strobeo(a);
    #1 $printtimescale;
    $printtimescale(tb);
    $finish;
  end
endmodule
