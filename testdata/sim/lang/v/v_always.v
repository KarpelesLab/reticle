`timescale 1ns/1ps
module tb;
  reg [3:0] a, b;
  reg [3:0] s1, s2;
  reg [3:0] v;
  reg clk, rst;
  reg [3:0] q, qn;
  integer np, nn, nb;
  always @* s1 = a + b;
  always @(*) s2 = a & b;
  always @(a or b) $display("%0t a-or-b a=%0d b=%0d", $time, a, b);
  always @(a, b) if (a == 4'd9) $display("%0t comma-list a=9", $time);
  always @(posedge v) np = np + 1;
  always @(negedge v) nn = nn + 1;
  always @(v[2]) nb = nb + 1;
  always @(posedge clk or posedge rst)
    if (rst) q <= 0; else q <= q + 1;
  always @(negedge clk) qn <= q;
  initial begin
    np = 0; nn = 0; nb = 0;
    clk = 0; rst = 1; v = 0;
    a = 1; b = 2;
    #1 $display("%0t s1=%0d s2=%0d", $time, s1, s2);
    a = 9;
    #1 $display("%0t s1=%0d s2=%0d", $time, s1, s2);
    v = 4'b0001; #1 v = 4'b0010; #1 v = 4'b0011; #1 v = 4'b0100; #1 v = 4'b0000;
    #1 v = 4'b1x01; #1 v = 4'b1x00; #1 v = 4'b1x0x; #1 v = 4'b0001;
    #1 $display("%0t np=%0d nn=%0d nb=%0d", $time, np, nn, nb);
    #1 rst = 0;
    repeat (4) begin #1 clk = 1; #1 clk = 0; end
    $display("%0t q=%0d qn=%0d", $time, q, qn);
    rst = 1; #1 $display("%0t async rst q=%0d", $time, q);
    rst = 0;
    clk = 1'bx; #1 clk = 1; #1 $display("%0t x->1 q=%0d", $time, q);
    clk = 0; #1 clk = 1'bz; #1 $display("%0t 0->z q=%0d", $time, q);
    $finish;
  end
endmodule
