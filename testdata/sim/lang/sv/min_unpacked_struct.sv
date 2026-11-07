`timescale 1ns/1ps
module tb;
  typedef struct { int a; logic [7:0] b; } rec_t;
  rec_t r;
  initial begin
    r.a = 10; r.b = 8'hAB;
    $display("M1 a=%0d b=%h", r.a, r.b);
    $finish;
  end
endmodule
