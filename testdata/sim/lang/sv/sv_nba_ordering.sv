`timescale 1ns/1ps
module tb;
  logic clk = 0;
  logic [3:0] a = 4'd1, b = 4'd2, c = 4'd3;
  logic [3:0] sr [3];
  always #5 clk = ~clk;
  always @(posedge clk) begin
    a <= b;
    b <= c;
    c <= a;
  end
  always @(posedge clk) begin
    sr[0] <= c;
    sr[1] <= sr[0];
    sr[2] <= sr[1];
  end
  initial begin
    sr[0] = 0; sr[1] = 0; sr[2] = 0;
    repeat (4) begin
      @(posedge clk);
      $display("NB pre-NBA t=%0t a=%0d b=%0d c=%0d", $time, a, b, c);
      #0 $display("NB #0 a=%0d", a);
      #1 $display("NB post a=%0d b=%0d c=%0d sr=%0d %0d %0d", a, b, c, sr[0], sr[1], sr[2]);
    end
    $strobe("NB strobe a=%0d", a);
    a <= 4'd9;
    $display("NB display a=%0d", a);
    #1;
    $finish;
  end
endmodule
