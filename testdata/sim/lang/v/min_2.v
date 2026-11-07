`timescale 1ns/1ps
module tb;
  reg signed [3:0] s; reg [7:0] w;
  initial begin
    s = -4'sd3;
    w = 1 ? s : 4'd0;  $display("a %b", w);
    w = 1 ? s : 4'sd0; $display("b %b", w);
    w = (s >>> 1) + 8'd0; $display("c %b", w);
    w = (s >>> 1); $display("d %b", w);
    w = s + 4'd0; $display("e %b", w);
    $finish;
  end
endmodule
