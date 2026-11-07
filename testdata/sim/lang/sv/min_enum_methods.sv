`timescale 1ns/1ps
module tb;
  typedef enum logic [1:0] {A, B, C} e_t;
  e_t e;
  initial begin
    e = B;
    $display("M1 name=%s first=%0d last=%0d num=%0d next=%0d prev=%0d", e.name(), e.first(), e.last(), e.num(), e.next(), e.prev());
    $finish;
  end
endmodule
