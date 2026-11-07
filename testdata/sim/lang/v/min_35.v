`timescale 1ns/1ps
module tb;
  initial begin
    $display("%b %b %b", "b" > "abc", 8'h62 > 24'h616263, "abc" == {16'h0, "abc"});
    $finish;
  end
endmodule
