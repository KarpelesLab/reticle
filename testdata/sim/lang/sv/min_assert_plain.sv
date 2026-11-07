`timescale 1ns/1ps
module tb;
  int a = 0;
  initial begin
    assert (a);
    assert (a) else $info("M2 info");
    $finish;
  end
endmodule
