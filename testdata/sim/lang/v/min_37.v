`timescale 1ns/1ps
module tb;
  real ra [0:1];
  initial begin
    ra[0] = 1.25; ra[1] = -2.5;
    $display("%f %f", ra[0], ra[1]);
    $finish;
  end
endmodule
