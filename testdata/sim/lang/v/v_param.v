`timescale 1ns/1ps
module sub #(parameter W = 4, parameter [7:0] V = 8'h11, parameter signed [7:0] S = -2,
             parameter real R = 1.5, parameter NAME = "sub") ();
  localparam L = W * 2;
  localparam integer CL = $clog2(W);
  initial #1 $display("%m W=%0d V=%h S=%0d R=%f NAME=%s L=%0d CL=%0d", W, V, S, R, NAME, L, CL);
endmodule
module sub2;
  parameter P = 1;
  parameter Q = P + 1;
  initial #2 $display("%m P=%0d Q=%0d", P, Q);
endmodule
module tb;
  parameter A = 5;
  parameter [3:0] B = 4'b1010;
  parameter signed [3:0] C = -3;
  parameter D = 3'b111;
  parameter E = -1;
  parameter [7:0] F = 300;
  parameter G = 2.5;
  parameter H = "hello";
  parameter I = A + B;
  parameter J = A > 3 ? 10 : 20;
  localparam K = $clog2(1000);
  localparam K0 = $clog2(0);
  localparam K1 = $clog2(1);
  localparam K2 = $clog2(2);
  localparam K3 = $clog2(33);
  parameter integer IP = 3.7;
  parameter real RP = 3;
  parameter M = 4'sd7 + 4'sd1;
  sub u0 ();
  sub #(8, 8'hff, 5, 2.25, "u1") u1 ();
  sub #(.NAME("u2"), .W(16)) u2 ();
  sub2 s0 ();
  sub2 #(.P(5)) s1 ();
  sub2 s2 ();
  defparam s2.P = 10;
  initial begin
    $display("A=%0d B=%b C=%0d C=%b D=%b E=%0d", A, B, C, C, D, E);
    $display("F=%0d G=%f H=%s I=%0d J=%0d", F, G, H, I, J);
    $display("K=%0d K0=%0d K1=%0d K2=%0d K3=%0d", K, K0, K1, K2, K3);
    $display("IP=%0d RP=%f M=%0d", IP, RP, M);
    $display("B+1=%0d C+1=%0d", B + 1, C + 1);
    $display("D width %b", D + 0);
    $display("H hex %h", H);
    $display("C ext %b", {4'b0, C});
    $display("C arith %0d", C * 2);
    #5 $finish;
  end
endmodule
