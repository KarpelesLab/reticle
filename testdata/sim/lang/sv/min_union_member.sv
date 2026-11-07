`timescale 1ns/1ps
module tb;
  typedef union packed {
    logic [15:0] w;
    logic [1:0][7:0] b;
  } u_t;
  u_t x;
  initial begin
    x.w = 16'hBEEF;
    $display("M1 x.b[1]=%h x.b[0]=%h", x.b[1], x.b[0]);
    x.b[1] = 8'h12;
    $display("M2 after x.b[1]=12: x.w=%h", x.w);
    $finish;
  end
endmodule
