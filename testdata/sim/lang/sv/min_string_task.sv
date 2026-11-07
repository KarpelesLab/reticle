`timescale 1ns/1ps
module tb;
  task automatic show(input string tag);
    $display("M1 tag=%s", tag);
  endtask
  initial begin
    show("abc");
    $finish;
  end
endmodule
