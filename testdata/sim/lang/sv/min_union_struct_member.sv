`timescale 1ns/1ps
module tb;
  typedef union packed {
    logic [15:0] w;
    struct packed { logic [7:0] hi; logic [7:0] lo; } s;
  } u_t;
  u_t x;
  initial begin
    x.w = 16'hBEEF;
    $display("M1 x.s.hi=%h x.s.lo=%h x.s=%h", x.s.hi, x.s.lo, x.s);
    $finish;
  end
endmodule
