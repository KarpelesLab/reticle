`timescale 1ns/1ps
module tb;
  logic clk = 0;
  logic rst;
  logic [3:0] a, b;
  logic [3:0] sum_c;
  logic [3:0] cnt;
  logic en;
  logic [3:0] lat;
  logic [3:0] k_const;
  logic [3:0] dep;
  always_comb sum_c = a + b;
  always_comb k_const = 4'd7;
  always_comb dep = sum_c ^ 4'hF;
  always_ff @(posedge clk or posedge rst)
    if (rst) cnt <= 4'd0;
    else cnt <= cnt + 4'd1;
  always_latch
    if (en) lat = a;
  always #5 clk = ~clk;
  initial begin
    $display("AK1 t=%0t k_const=%0d sum_c=%b", $time, k_const, sum_c);
    #0 $display("AK2 t0 after #0 k_const=%0d", k_const);
    a = 4'd3; b = 4'd4; rst = 1; en = 0;
    #1 $display("AK3 sum_c=%0d dep=%h cnt=%0d lat=%b", sum_c, dep, cnt, lat);
    rst = 0;
    en = 1;
    #1 $display("AK4 lat=%0d", lat);
    en = 0; a = 4'd9;
    #1 $display("AK5 lat held=%0d sum_c=%0d", lat, sum_c);
    repeat (3) @(posedge clk);
    #1 $display("AK6 cnt=%0d t=%0t", cnt, $time);
    en = 1;
    #1 $display("AK7 lat=%0d", lat);
    $finish;
  end
endmodule
