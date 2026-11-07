`timescale 1ns/1ps
module tb;
  string s = "init";
  initial begin
    $display("M1 s=%s", s);
    $finish;
  end
endmodule
