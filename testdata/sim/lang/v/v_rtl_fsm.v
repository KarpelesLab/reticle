`timescale 1ns/1ps
// Sequence detector for 1011 (overlapping), Mealy output and Moore state.
module det1011 (input clk, input rst, input x, output reg hit, output [1:0] st);
  localparam S0 = 2'd0, S1 = 2'd1, S10 = 2'd2, S101 = 2'd3;
  reg [1:0] state, nxt;
  assign st = state;
  always @(posedge clk) if (rst) state <= S0; else state <= nxt;
  always @* begin
    nxt = state; hit = 1'b0;
    case (state)
      S0:   nxt = x ? S1 : S0;
      S1:   nxt = x ? S1 : S10;
      S10:  nxt = x ? S101 : S0;
      S101: begin
        if (x) begin nxt = S1; hit = 1'b1; end
        else nxt = S10;
      end
      default: nxt = S0;
    endcase
  end
endmodule
module tb;
  reg clk, rst, x;
  wire hit;
  wire [1:0] st;
  reg [31:0] pattern;
  reg [3:0] hist;
  integer i, hits, exp_hits;
  det1011 dut (.clk(clk), .rst(rst), .x(x), .hit(hit), .st(st));
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin
    pattern = 32'b1011_0110_1101_1011_1011_0001_0110_1011;
    hits = 0; exp_hits = 0; hist = 0;
    rst = 1; x = 0;
    @(posedge clk); #1 rst = 0;
    for (i = 31; i >= 0; i = i - 1) begin
      x = pattern[i];
      #1;
      hist = {hist[2:0], x};
      if (hist == 4'b1011 && i <= 28) exp_hits = exp_hits + 1;
      if (hit) hits = hits + 1;
      $display("%0t i=%0d x=%b st=%0d hit=%b", $time, i, x, st, hit);
      @(posedge clk); #1;
    end
    $display("hits=%0d expected=%0d %s", hits, exp_hits, hits == exp_hits ? "PASS" : "FAIL");
    $finish;
  end
endmodule
