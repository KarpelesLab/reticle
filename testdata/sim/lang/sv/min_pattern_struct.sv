`timescale 1ns/1ps
module tb;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
  pair_t pr;
  initial begin
    pr = '{4'h1, 4'h2};
    $display("M1 pr=%h", pr);
    $finish;
  end
endmodule
