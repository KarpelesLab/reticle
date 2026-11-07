`timescale 1ns/1ps
module tb;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
  pair_t pr;
  initial begin
    pr = 8'h80;
    pr.hi[0] = 1'b1;
    $display("M1 bit write pr=%h", pr);
    pr.lo[3:2] = 2'b01;
    $display("M2 part write pr=%h", pr);
    $finish;
  end
endmodule
