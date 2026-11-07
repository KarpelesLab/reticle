// `$printtimescale` as xsim 2026.1 prints it (`printtimescale_tb.xsim`):
// with no argument the calling module, by its module name; with one, the
// instance it names, by its path. Each module prints in its own initial
// block after a delay, so the order does not depend on which runs first.
`timescale 1us/10ns
module sub;
  initial #2 $printtimescale;
endmodule
`timescale 1ns/1ps
module tb;
  sub u1();
  initial begin
    $printtimescale;
    $printtimescale(u1);
    $printtimescale(tb);
    #5000 $finish;
  end
endmodule
