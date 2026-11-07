`timescale 1ns/1ps
module tb;
  integer seed, r;
  initial begin
    seed = 1;
    r = $random(seed); $display("r=%0d seed=%0d", r, seed);
    r = $random(seed); $display("r=%0d seed=%0d", r, seed);
    r = $random; $display("noseed r=%0d", r);
    seed = 5;
    r = $dist_uniform(seed, 0, 99); $display("dist_uniform r=%0d seed=%0d", r, seed);
    $finish;
  end
endmodule
