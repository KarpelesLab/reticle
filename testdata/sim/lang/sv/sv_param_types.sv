`timescale 1ns/1ps
typedef struct packed { logic [3:0] a; logic [3:0] b; } cfg_t;

module holder #(
  parameter type T = logic [7:0],
  parameter int N = 4,
  parameter string NAME = "dflt",
  parameter real SCALE = 1.5,
  parameter cfg_t CFG = '{a: 4'h1, b: 4'h2},
  parameter logic [7:0] INIT = 8'h5A
) (output T q);
  localparam int NB = $bits(T);
  initial begin
    q = T'(INIT);
    $display("PT %s: bits(T)=%0d N=%0d SCALE=%f CFG=%h CFG.a=%0d q=%h", NAME, NB, N, SCALE, CFG, CFG.a, q);
  end
endmodule

module tb;
  function automatic int fib(int n);
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
  endfunction
  localparam int F10 = fib(10);
  localparam int W = $clog2(F10);
  localparam logic [W-1:0] MAXV = '1;
  parameter int unsigned UP = 32'hFFFF_FFFF;
  parameter P_IMPLICIT = 3'b101;
  logic [7:0] q0;
  logic [15:0] q1;
  logic [2:0] q2;
  int q3;
  holder h0 (.q(q0));
  holder #(.T(logic [15:0]), .N(7), .NAME("wide"), .SCALE(0.25), .CFG('{a: 4'hC, b: 4'hD}), .INIT(8'hF0)) h1 (.q(q1));
  holder #(logic [2:0], 2, "narrow") h2 (.q(q2));
  holder #(.T(int), .NAME("int")) h3 (.q(q3));
  initial begin
    #1;
    $display("PT1 F10=%0d W=%0d MAXV=%b UP=%0d PI=%b bits(PI)=%0d", F10, W, MAXV, UP, P_IMPLICIT, $bits(P_IMPLICIT));
    $display("PT2 q0=%h q1=%h q2=%b q3=%0d", q0, q1, q2, q3);
    $finish;
  end
endmodule
