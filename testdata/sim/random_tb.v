// `$random` and the `$dist_*` functions as xsim 2026.1 computes them
// (`random_tb.xsim`), which is IEEE 1364-2005's own code: the values, the
// seeds the calls leave in their variables, and the seed `$random` with no
// argument goes on from. No call here makes xsim warn.
`timescale 1ns/1ps
module tb;
  integer seed, r, i, s2;
  reg [3:0] small4;
  initial begin
    r = $random; $display("A %0d", r);
    seed = 1; r = $random(seed); $display("s %0d %0d", r, seed);
    r = $random(seed); $display("s %0d %0d", r, seed);
    r = $random; $display("B %0d", r);
    r = $dist_uniform(seed, 0, 99); $display("u %0d %0d", r, seed);
    r = $random; $display("C %0d", r);
    r = $dist_normal(seed, 0, 10); $display("n %0d %0d", r, seed);
    r = $random; $display("D %0d", r);
    r = $dist_poisson(seed, 30); $display("p %0d %0d", r, seed);
    r = $random; $display("E %0d", r);
    s2 = 1000; r = $random(s2); $display("s2 %0d %0d", r, s2);
    r = $random; $display("G %0d", r);
    seed = 0; r = $random(seed); $display("seed0 %0d %0d", r, seed);
    seed = -7; r = $random(seed); $display("seedm7 %0d %0d", r, seed);
    small4 = $random(seed); $display("small4 %0d %0d", small4, seed);
    seed = 3; r = $dist_uniform(seed, -10, 10); $display("u1 %0d %0d", r, seed);
    seed = 11;
    for (i = 0; i < 3; i = i + 1) begin r = $dist_normal(seed, 100, 15); $display("n %0d %0d", r, seed); end
    seed = 11;
    for (i = 0; i < 3; i = i + 1) begin r = $dist_exponential(seed, 50); $display("e %0d %0d", r, seed); end
    seed = 11;
    for (i = 0; i < 3; i = i + 1) begin r = $dist_poisson(seed, 7); $display("p %0d %0d", r, seed); end
    seed = 11;
    for (i = 0; i < 3; i = i + 1) begin r = $dist_chi_square(seed, 4); $display("c %0d %0d", r, seed); end
    seed = 11;
    for (i = 0; i < 3; i = i + 1) begin r = $dist_t(seed, 3); $display("t %0d %0d", r, seed); end
    seed = 11;
    for (i = 0; i < 3; i = i + 1) begin r = $dist_erlang(seed, 3, 20); $display("er %0d %0d", r, seed); end
    r = $random; $display("H %0d", r);
    $finish;
  end
endmodule
