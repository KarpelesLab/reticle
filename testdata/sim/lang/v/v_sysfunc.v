`timescale 1ns/1ps
module tb;
  integer i;
  reg [7:0] r;
  reg signed [7:0] s;
  real x;
  initial begin
    for (i = 0; i < 10; i = i + 1) $display("clog2(%0d)=%0d", i, $clog2(i));
    $display("clog2 big %0d %0d %0d", $clog2(256), $clog2(257), $clog2(32'hffffffff));
    r = 8'hf0; s = $signed(r); $display("signed %0d", s);
    $display("signed expr %0d", $signed(r) + 0);
    $display("signed expr2 %0d", $signed(r) + 8'sd0);
    $display("unsigned %0d", $unsigned(-8'sd1));
    $display("unsigned ctx %h", $unsigned(-8'sd1) + 16'd0);
    $display("signed ctx %h", $signed(8'hff) + 16'sd0);
    $display("signed of 1bit %0d", $signed(1'b1));
    $display("signed width %b", $signed(4'b1001));
    #3;
    $display("time %0d stime %0d realtime %f", $time, $stime, $realtime);
    $display("time arith %0d", $time * 2 + 1);
    x = $realtime / 2; $display("realtime/2 %f", x);
    $display("rtoi %0d itor %f", $rtoi(-3.99), $itor(5));
    $display("realtobits %h", $realtobits(0.1));
    $display("bitstoreal %f", $bitstoreal($realtobits(2.75)));
    $display("$test$plusargs %0d", $test$plusargs("nonexistent"));
    $finish;
  end
endmodule
