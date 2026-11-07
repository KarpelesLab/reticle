`timescale 1ns/1ps
// sv_hier_task without hierarchical references into a child instance; %m only.
module child;
  int val = 3;
  initial #1 $display("HT0 child %%m=%m val=%0d", val);
endmodule

module tb;
  child c1 ();
  child c2 ();
  task automatic where();
    $display("HT4 task %%m=%m");
  endtask
  initial begin
    #2;
    $display("HT3 %%m in tb: %m");
    where();
    begin : named_blk
      $display("HT5 named block %%m=%m");
    end
    $finish;
  end
endmodule
