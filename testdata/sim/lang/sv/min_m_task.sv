`timescale 1ns/1ps
module tb;
  task automatic where();
    $display("M1 %m");
  endtask
  function automatic int fwhere();
    $display("M2 %m");
    return 0;
  endfunction
  initial begin
    where();
    void'(fwhere());
    begin : blk
      $display("M3 %m");
    end
    $finish;
  end
endmodule
