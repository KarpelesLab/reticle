`timescale 1ns/1ps
module tb;
  string s, u;
  initial begin
    s = "ab";
    u = {s, "cd"};
    $display("M1 u=%s", u);
    $finish;
  end
endmodule
