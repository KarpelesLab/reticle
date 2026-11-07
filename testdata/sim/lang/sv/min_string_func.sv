`timescale 1ns/1ps
module tb;
  function automatic int slen(input string s);
    return s.len();
  endfunction
  initial begin
    $display("M1 len=%0d", slen("abcdef"));
    $finish;
  end
endmodule
