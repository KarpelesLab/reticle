`timescale 1ns/1ps
module tb;
  reg [7:0] a, b, c, d;
  reg clk;
  reg [7:0] q1, q2, q3;
  initial begin
    a = 1; b = 2;
    a <= b; b <= a;
    $display("%0t before nba a=%0d b=%0d", $time, a, b);
    #0 $display("%0t after #0 a=%0d b=%0d", $time, a, b);
    #1 $display("%0t after #1 a=%0d b=%0d", $time, a, b);
    c = 0;
    c <= 1; c <= 2; c <= 3;
    #1 $display("%0t last nba wins c=%0d", $time, c);
    d = 5;
    d <= #2 10;
    d = #1 20;
    $display("%0t intra blocking d=%0d", $time, d);
    #2 $display("%0t intra nba d=%0d", $time, d);
    a = 7;
    b = #3 a + 1;
    $display("%0t b=%0d", $time, b);
    a = 1;
    a <= #1 a + 1;
    a = 50;
    #2 $display("%0t a=%0d (sampled rhs)", $time, a);
    repeat (3) begin
      a <= a + 1;
      #1;
    end
    $display("%0t loop a=%0d", $time, a);
    $finish;
  end
  initial begin
    clk = 0; q1 = 0; q2 = 0; q3 = 0;
    forever #5 clk = ~clk;
  end
  always @(posedge clk) begin
    q1 <= q1 + 1;
    q2 <= q1;
    q3 = q1;
  end
  always @(posedge clk) $display("%0t clk q1=%0d q2=%0d q3=%0d", $time, q1, q2, q3);
  initial #11 $display("%0t sample q1=%0d q2=%0d q3=%0d", $time, q1, q2, q3);
endmodule
