`timescale 1ns/1ps
module tb;
  int q [$];
  initial begin
    q.push_back(3);
    $display("M1 size=%0d q0=%0d", q.size(), q[0]);
    $finish;
  end
endmodule
