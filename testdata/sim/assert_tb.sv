// Immediate assertions and the severity tasks, as xsim 2026.1 prints them
// (`assert_tb.xsim`, its output less the `Time: ... File: ...` line it
// adds after each report): a pass action runs only on a pass, an else
// action only on a failure, an unknown condition fails, and a failure
// with no action prints `Error: Assertion violation`.
`timescale 1ns/1ps
module tb;
  int a = 0;
  logic u = 1'bx;
  initial begin
    assert (a) $display("A1 pass");
    assert (a == 0) $display("A2 pass"); else $display("A2 fail");
    assert (a) else begin $display("A3 else1"); $display("A3 else2"); end
    assert (a == 0) else $error("A4 not shown");
    assert (a) else $error;
    assert (a) else $warning("A6 %0d", a);
    assert (a) else $info;
    assert (a) else $error("A8 %0d %s", 5, "str");
    assert (u) $display("A9 pass"); else $display("A9 fail");
    assert (u);
    assert (a == 0);
    $error("E1");
    $warning("W1");
    $info("I1");
    $error;
    $warning;
    $info;
    $display("after");
    #1 $fatal(1, "F1 %0d", 3);
    $display("not reached");
  end
endmodule
