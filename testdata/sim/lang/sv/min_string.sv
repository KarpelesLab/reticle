`timescale 1ns/1ps
module tb;
  string s;
  initial begin
    s = "Hello";
    $display("M1 s=%s len=%0d", s, s.len());
    $finish;
  end
endmodule
