`timescale 1ns/1ps
module tb;
  integer n, c;
  initial begin
    c = 0; n = -1;
    repeat (n) begin c = c + 1; if (c > 5) $finish; end
    $display("repeat(-1) ran %0d times", c);
    $finish;
  end
endmodule
