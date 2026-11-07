`timescale 1ns/1ps
typedef struct packed { logic [3:0] a; logic [3:0] b; } cfg_t;
module sub #(parameter cfg_t CFG = '{a: 4'h1, b: 4'h2}) ();
  initial $display("M1 %m CFG=%h", CFG);
endmodule
module tb;
  sub s0 ();
  sub #(.CFG('{a: 4'hC, b: 4'hD})) s1 ();
  initial #1 $finish;
endmodule
