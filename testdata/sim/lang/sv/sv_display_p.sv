`timescale 1ns/1ps
module tb;
  typedef enum logic [1:0] {RED, GRN, BLU} col_t;
  typedef struct packed { logic [3:0] a; logic signed [3:0] b; } ps_t;
  typedef struct packed { col_t c; ps_t inner; logic f; } nest_t;
  ps_t p;
  nest_t n;
  col_t c;
  logic [7:0] arr [3];
  int ia [2];
  logic [3:0][3:0] pk;
  initial begin
    p = 8'h3F;
    $display("DP1 %p", p);
    n = {BLU, 8'hA9, 1'b1};
    $display("DP2 %p", n);
    c = GRN;
    $display("DP3 %p", c);
    arr[0] = 8'd1; arr[1] = 8'd2; arr[2] = 8'hFF;
    $display("DP4 %p", arr);
    ia[0] = -1; ia[1] = 7;
    $display("DP5 %p", ia);
    pk = 16'h1234;
    $display("DP6 %p", pk);
    $display("DP7 %0p", p);
    $finish;
  end
endmodule
