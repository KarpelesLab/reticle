`timescale 1ns/1ps
module child;
  int val = 3;
  function automatic int get2(); return val * 2; endfunction
  task automatic setv(input int v); val = v; endtask
endmodule

module tb;
  child c1 ();
  child c2 ();
  initial begin
    $display("HT1 c1.val=%0d c2.get2=%0d", c1.val, c2.get2());
    c1.setv(10);
    c2.val = 20;
    $display("HT2 c1.val=%0d c2.val=%0d c1.get2=%0d", c1.val, c2.val, c1.get2());
    $display("HT3 %%m in tb: %m");
    $finish;
  end
endmodule
