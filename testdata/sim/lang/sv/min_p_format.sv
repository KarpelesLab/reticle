`timescale 1ns/1ps
module tb;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } ps_t;
  ps_t p;
  logic [7:0] v;
  initial begin
    p = 8'h3F;
    v = 8'h3F;
    $display("M1 %p", p);
    $display("M2 %p", v);
    $finish;
  end
endmodule
