`timescale 1ns/1ps
module child;
  function automatic int get7(); return 7; endfunction
endmodule
module tb;
  child c1 ();
  initial begin
    $display("M1 c1.get7=%0d", c1.get7());
    $finish;
  end
endmodule
