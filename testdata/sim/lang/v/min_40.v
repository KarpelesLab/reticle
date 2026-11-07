`timescale 1ns/1ps
module tb;
  reg [1:0] a, b;
  reg en;
  wire [1:0] y;
  wire nm;
  and ga [1:0] (y, a, b);
  nmos (nm, a[0], en);
  initial begin
    a = 2'b11; b = 2'b10; en = 1;
    #1 $display("y=%b nm=%b", y, nm);
    $finish;
  end
endmodule
