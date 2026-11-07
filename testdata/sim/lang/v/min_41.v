`timescale 1ns/1ps
primitive inv (o, i);
  output o; input i;
  table
    0 : 1;
    1 : 0;
  endtable
endprimitive
module tb;
  reg a;
  wire y;
  inv u (y, a);
  initial begin
    a = 0; #1 $display("y=%b", y);
    $finish;
  end
endmodule
