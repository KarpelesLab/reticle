`timescale 1ns/1ps
module seq_detect (
  input  logic clk,
  input  logic rst_n,
  input  logic din,
  output logic hit,
  output logic [2:0] st_out
);
  typedef enum logic [2:0] {S0, S1, S10, S101, S1011} state_t;
  state_t state, next;
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) state <= S0;
    else state <= next;
  always_comb begin
    next = state;
    unique case (state)
      S0:    next = din ? S1 : S0;
      S1:    next = din ? S1 : S10;
      S10:   next = din ? S101 : S0;
      S101:  next = din ? S1011 : S10;
      S1011: next = din ? S1 : S10;
      default: next = S0;
    endcase
  end
  assign hit = (state == S1011);
  assign st_out = state;
endmodule

module tb;
  logic clk = 0, rst_n = 0, din = 0, hit;
  logic [2:0] st_out;
  logic [15:0] pattern = 16'b1011_0110_1101_1011;
  int hits = 0;
  seq_detect dut (.*);
  always #5 clk = ~clk;
  initial begin
    @(negedge clk);
    rst_n = 1;
    for (int i = 15; i >= 0; i--) begin
      din = pattern[i];
      @(posedge clk); #1;
      if (hit) hits++;
      $display("SD bit=%0d din=%b st=%0d hit=%b", i, din, st_out, hit);
    end
    $display("SD total hits=%0d", hits);
    $finish;
  end
endmodule
