`timescale 1ns/1ps
module tb;
  reg [3:0] a;
  wire [3:0] y5;
  wire [3:0] #2 y6;
  wire [3:0] y2;
  assign #1 y5 = a;
  assign y6 = a;
  assign #(1, 3) y2 = a;
  initial begin
    $monitor("%0t a=%0d y5=%0d y6=%0d y2=%0d", $time, a, y5, y6, y2);
    a = 4'd0;
    #5 a = 4'd1;
    #5 a = 4'd2; #0.5 a = 4'd3;
    #5 a = 4'd0;
    #5 $finish;
  end
endmodule
