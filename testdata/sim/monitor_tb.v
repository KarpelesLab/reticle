// `$monitor` as xsim 2026.1 runs it (`monitor_tb.xsim`): a line at the end
// of a time step in which a monitored value changed, and at no other:
// not for `$time` alone, not for an operand change that leaves the
// expression's value (`a + b`), not for a value changed and changed back
// in one step, not for an unmonitored variable, and not while
// `$monitoroff` holds, after which a change made meanwhile prints.
`timescale 1ns/1ps
module tb;
  reg [7:0] a, b, c, d;
  initial begin
    a = 0; b = 0; c = 0; d = 0;
    $monitorh("%0t mon a+b=%0d c=%0d rt=%0t", $time, a + b, c, $realtime, " ", c);
    #1 a = 1;
    #1 begin a = 0; b = 1; end
    #1 begin a = 2; a = 0; end
    #1 c = 1;
    #1 c = 2; #0 c = 1;
    #1 $monitoroff; c = 5;
    #1 $monitoron;
    #1 c = 5;
    #1 d = 1;
    #1 c = 5;
    #1 $monitoroff;
    #1 $monitoron;
    #1 $display("%0t display", $time);
    #1 c = 8'h1f;
    #1 $finish;
  end
endmodule
