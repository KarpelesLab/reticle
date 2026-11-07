`timescale 1ns/1ps
module tb;
  integer seed, i, r;
  reg [7:0] b;
  initial begin
    seed = 1;
    for (i = 0; i < 5; i = i + 1) begin
      r = $random(seed);
      $display("random(seed) %0d seed=%0d", r, seed);
    end
    seed = 42;
    for (i = 0; i < 3; i = i + 1) $display("mod %0d", $random(seed) % 10);
    seed = 7;
    for (i = 0; i < 3; i = i + 1) begin b = $random(seed); $display("byte %h", b); end
    seed = 3;
    for (i = 0; i < 3; i = i + 1) $display("urandom-ish %0d", {$random(seed)} % 100);
    seed = 5;
    for (i = 0; i < 3; i = i + 1) $display("dist_uniform %0d", $dist_uniform(seed, 0, 99));
    seed = 0;
    $display("seed0 %0d %0d", $random(seed), seed);
    seed = -1;
    $display("seedm1 %0d %0d", $random(seed), seed);
    $finish;
  end
endmodule
