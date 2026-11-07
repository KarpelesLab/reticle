`timescale 1ns/1ps
package p1;
  typedef enum {IDLE, BUSY, DONE} st_t;
  parameter int K = 3;
  function automatic int twice(int x); return 2 * x; endfunction
endpackage
package p2;
  parameter int K = 100;
  parameter logic [7:0] MASK = 8'h0F;
endpackage

module sub import p1::*; (input st_t s, output int o);
  always_comb o = twice(int'(s)) + K;
endmodule

module tb;
  import p1::st_t;
  import p1::IDLE;
  import p1::BUSY;
  import p2::MASK;
  st_t s;
  int o;
  sub u (.s(s), .o(o));
  initial begin
    s = IDLE;
    #1 $display("PI1 s=%0d o=%0d", s, o);
    s = BUSY;
    #1 $display("PI2 s=%0d o=%0d", s, o);
    s = p1::DONE;
    #1 $display("PI3 s=%0d o=%0d", s, o);
    $display("PI4 MASK=%h p2::K=%0d p1::K=%0d", MASK, p2::K, p1::K);
    $display("PI5 twice=%0d", p1::twice(21));
    $finish;
  end
endmodule
