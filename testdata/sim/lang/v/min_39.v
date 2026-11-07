`timescale 1ns/1ps
module tb;
  reg a, b;
  wire t;
  wand wa;
  assign t = a; assign t = b;
  assign wa = a; assign wa = b;
  initial begin
    a = 0; b = 1;
    #1 $display("wire=%b wand=%b", t, wa);
    $finish;
  end
endmodule
