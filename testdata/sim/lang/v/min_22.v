`timescale 1ns/1ps
module tb;
  reg e1, e2;
  wire tr;
  assign tr = e1 ? 1'b1 : 1'bz;
  assign tr = e2 ? 1'b0 : 1'bz;
  initial begin
    e1 = 0; e2 = 0; #1 $display("none %b", tr);
    e1 = 1; e2 = 0; #1 $display("e1 %b", tr);
    e1 = 0; e2 = 1; #1 $display("e2 %b", tr);
    e1 = 1; e2 = 1; #1 $display("both %b", tr);
    $finish;
  end
endmodule
