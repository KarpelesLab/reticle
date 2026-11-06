`timescale 1ns / 1ps
// The global set/reset under IDDR: a clock from time zero, an asynchronous
// set pulse inside the first 100 ns and one just before its end, and the
// outputs printed every half nanosecond. Nothing moves until the reset ends.
module tb;
  reg c = 1'b0;
  reg d = 1'b1, s = 1'b0;
  wire [3:0] q;
  IDDR #(.SRTYPE("SYNC")) u0 (.Q1(q[0]), .Q2(q[1]), .C(c), .CE(1'b1), .D(d), .R(1'b0), .S(s));
  IDDR #(.SRTYPE("ASYNC")) u1 (.Q1(q[2]), .Q2(q[3]), .C(c), .CE(1'b1), .D(d), .R(1'b0), .S(s));
  always #3 c = ~c;
  initial begin
    #10 s = 1;
    #10 s = 0;
    #79.5 s = 1;
    #0.4 s = 0;
  end
  integer i;
  initial begin
    for (i = 0; i < 230; i = i + 1) begin
      #0.5 $display("%0d %b %b %b", i, c, s, q);
    end
    $finish;
  end
endmodule
