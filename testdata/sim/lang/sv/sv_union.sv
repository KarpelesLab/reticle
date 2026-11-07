`timescale 1ns/1ps
module tb;
  typedef union packed {
    logic [15:0] w;
    logic [1:0][7:0] b;
    struct packed { logic [7:0] hi; logic [7:0] lo; } s;
  } u_t;
  typedef union packed signed {
    logic [7:0] u;
    logic [7:0] v;
  } su_t;
  u_t x;
  su_t y;
  int i;
  initial begin
    x.w = 16'hBEEF;
    $display("N1 w=%h b1=%h b0=%h hi=%h lo=%h", x.w, x.b[1], x.b[0], x.s.hi, x.s.lo);
    x.s.lo = 8'h00;
    $display("N2 w=%h", x.w);
    x.b[1] = 8'h12;
    $display("N3 w=%h s=%p", x.w, x.s);
    $display("N4 bits=%0d", $bits(x));
    x = 16'h1234;
    $display("N5 x=%h", x);
    y = 8'h80;
    i = y;
    $display("N6 y=%0d i=%0d", y, i);
    $finish;
  end
endmodule
