`timescale 1ns/1ps
module tb;
  reg [1:0] d1, d2;
  wire [1:0] t;
  wand [1:0] wa;
  wor [1:0] wo;
  wire undriven;
  wire [3:0] multi;
  assign t = d1; assign t = d2;
  assign wa = d1; assign wa = d2;
  assign wo = d1; assign wo = d2;
  assign multi[1:0] = d1;
  assign multi[3:2] = d2;
  integer i, j;
  reg [1:0] vals [0:3];
  initial begin
    vals[0] = 2'b00; vals[1] = 2'b01; vals[2] = 2'b1x; vals[3] = 2'bz1;
    for (i = 0; i < 4; i = i + 1)
      for (j = 0; j < 4; j = j + 1) begin
        d1 = vals[i]; d2 = vals[j];
        #1 $display("d1=%b d2=%b wire=%b wand=%b wor=%b", d1, d2, t, wa, wo);
      end
    d1 = 2'b10; d2 = 2'b01;
    #1 $display("undriven %b multi %b", undriven, multi);
    $finish;
  end
endmodule
