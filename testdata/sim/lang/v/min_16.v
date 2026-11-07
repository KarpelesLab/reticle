`timescale 1ns/1ps
module tb;
  real r;
  initial begin
    r = 3.14159;
    $display("[%-10.3f] [%10.3f] [%-8d] [%-6h]", r, r, 8'd5, 8'hab);
    r = 1.0;
    $display("[%d]", r);
    $finish;
  end
endmodule
