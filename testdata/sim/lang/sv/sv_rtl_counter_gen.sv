`timescale 1ns/1ps
module up_counter #(parameter int W = 4, parameter logic [W-1:0] MAX = '1) (
  input logic clk, rst, en,
  output logic [W-1:0] q,
  output logic wrap
);
  always_ff @(posedge clk)
    if (rst) q <= '0;
    else if (en) q <= (q == MAX) ? '0 : q + 1'b1;
  assign wrap = en && (q == MAX);
endmodule

module chain #(parameter int N = 3, parameter int W = 2) (
  input logic clk, rst,
  output logic [N*W-1:0] q
);
  logic [N:0] carry;
  assign carry[0] = 1'b1;
  for (genvar i = 0; i < N; i++) begin : g_cnt
    up_counter #(.W(W)) c (.clk, .rst, .en(carry[i]), .q(q[i*W +: W]), .wrap(carry[i+1]));
  end
endmodule

module tb;
  logic clk = 0, rst = 1;
  logic [5:0] q;
  logic [3:0] q10;
  logic w10;
  chain #(.N(3), .W(2)) dut (.clk, .rst, .q);
  up_counter #(.W(4), .MAX(4'd9)) dec (.clk, .rst, .en(1'b1), .q(q10), .wrap(w10));
  always #5 clk = ~clk;
  initial begin
    @(negedge clk); rst = 0;
    for (int i = 0; i < 20; i++) begin
      @(negedge clk);
      $display("CG i=%0d q=%b (%0d) dec=%0d wrap=%b", i, q, q, q10, w10);
    end
    $finish;
  end
endmodule
