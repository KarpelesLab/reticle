// `%m` names the scope a statement is written in, as xsim 2026.1 prints it
// (`scope_tb.xsim`): a named block, a nested one, a task or a function,
// in the top module and in an instance; `%%m` is not a conversion. And
// `$test$plusargs` finds nothing when no plusargs are given.
`timescale 1ns/1ps
module sub;
  task t1;
    $display("T %m");
  endtask
  initial #1 begin : sblk
    $display("S %m");
    t1;
    begin : inner
      $display("I %m %%m %s", $sformatf("%m"));
      t1;
    end
  end
endmodule
module tb;
  integer v, n;
  sub u1();
  function integer f(input integer x);
    begin
      $display("F %m");
      f = x + 1;
    end
  endfunction
  initial begin
    $display("A %m");
    begin : blk
      $display("B %m");
      v = 1;
      v = f(v);
      for (n = 0; n < 1; n = n + 1) begin : lp
        $display("L %m");
      end
    end
    $display("P %0d %0d", $test$plusargs("none"), $value$plusargs("n=%d", n));
    #2 $finish;
  end
endmodule
