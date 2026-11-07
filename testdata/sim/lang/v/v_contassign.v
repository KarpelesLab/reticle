`timescale 1ns/1ps
module tb;
  reg [3:0] a, b;
  wire [3:0] y1, y2, y3, y4;
  wire [3:0] y5;
  wire [3:0] #2 y6;
  wire [7:0] wide;
  wire [1:0] narrow;
  wire impl;
  assign #3 y1 = a + b;
  assign #(1, 2) y2 = a & b;
  assign y3 = a | b, y4 = a ^ b;
  assign #1 y5 = a;
  assign y6 = b;
  assign wide = a;
  assign narrow = a + b;
  assign impl_net = a[0];
  wire [3:0] decl = a - b;
  initial begin
    $monitor("%0t a=%0d b=%0d y1=%0d y2=%0d y3=%0d y4=%0d y5=%0d y6=%0d", $time, a, b, y1, y2, y3, y4, y5, y6);
    a = 1; b = 2;
    #5 a = 3;
    #1 a = 4;
    #1 a = 5;
    #5 b = 7;
    #10;
    $display("wide=%h narrow=%b decl=%0d impl=%b", wide, narrow, decl, impl_net);
    a = 4'd1; #0.5 a = 4'd2; #0.5 a = 4'd3;
    #5 $display("glitch y5=%0d", y5);
    $finish;
  end
endmodule
