module sub;
  timeunit 10ns;
  timeprecision 1ns;
  initial #2 $display("M1 sub t=%0t", $time);
endmodule
module tb;
  timeunit 1ns;
  timeprecision 1ps;
  sub s ();
  initial #30 $finish;
endmodule
