module sub;
  timeunit 10ns;
  timeprecision 1ns;
  initial begin
    #2 $display("TU1 sub t=%0t realtime=%0t", $time, $realtime);
    #1.5 $display("TU2 sub t=%0t", $time);
  end
endmodule

module tb;
  timeunit 1ns;
  timeprecision 1ps;
  sub s ();
  initial begin
    #5 $display("TU3 tb t=%0t", $time);
    #0.5 $display("TU4 tb t=%0t realtime=%0t", $time, $realtime);
    #40 $display("TU5 tb t=%0t", $time);
    $printtimescale;
    $finish;
  end
endmodule
