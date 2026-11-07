`timescale 1ns/1ps
module tb;
  reg [8*4:1] s;
  initial begin
    s = "ab";
    $display("[%s]", s);
    s = 32'h41004200;
    $display("[%s]", s);
    $display("[%s]", 8'bx);
    $display("l %l");
    $finish;
  end
endmodule
