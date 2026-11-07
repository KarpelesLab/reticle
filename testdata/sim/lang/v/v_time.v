`timescale 1ns/1ps
module tb;
  time t;
  integer i;
  initial begin
    $display("t0 %t %0t %d", $time, $time, $time);
    #1 $display("t1 %t %0t", $time, $time);
    #1.5 $display("t2.5 %t %0t realtime %f", $time, $time, $realtime);
    #0.001 $display("t+ps %0t %f", $time, $realtime);
    #0.0004 $display("t+sub-ps %f", $realtime);
    #10 $display("stime %0d time %0d", $stime, $time);
    t = $time; $display("t var %0d %h", t, t);
    i = $time; $display("i var %0d", i);
    $timeformat(-9, 3, " ns", 12);
    $display("tf [%t]", $time);
    $display("tf real [%t]", $realtime);
    $timeformat(-12, 0, " ps", 0);
    $display("tf ps [%t]", $time);
    $timeformat(-6, 6, "us", 15);
    $display("tf us [%t]", $realtime);
    #100;
    $display("after 100 [%t]", $time);
    $printtimescale;
    #(2 * 3) $display("expr delay %0t", $time);
    i = 4; #i $display("var delay %0t", $time);
    #(0.25 * 4) $display("real expr delay %0t", $time);
    $finish;
  end
endmodule
