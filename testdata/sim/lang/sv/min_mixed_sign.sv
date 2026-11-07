`timescale 1ns/1ps
module tb;
  logic signed [7:0] sa;
  logic [7:0] ua;
  logic [15:0] sw;
  initial begin
    sa = -8'sd5; ua = 8'd5;
    sw = sa + ua;
    $display("M1 sa+ua (16-bit ctx) = %h (xsim expects 0100)", sw);
    $finish;
  end
endmodule
