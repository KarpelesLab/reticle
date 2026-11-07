`timescale 1ns/1ps
module tb;
  reg [7:0] u;
  reg signed [7:0] s;
  reg [3:0] n;
  reg [15:0] w;
  reg signed [15:0] sw;
  integer i;
  initial begin
    u = 8'b1001_0110; s = -8'sd106;
    $display("u << 2 %b  >> 2 %b", u << 2, u >> 2);
    $display("u <<< 2 %b >>> 2 %b", u <<< 2, u >>> 2);
    $display("s >>> 2 %b %d", s >>> 2, s >>> 2);
    $display("s >> 2 %b %d", s >> 2, s >> 2);
    $display("s <<< 1 %b %d", s <<< 1, s <<< 1);
    $display("shift by 8 %b %b", u << 8, s >>> 8);
    $display("shift by 9 %b", s >>> 9);
    $display("shift big %b %b", u << 100, s >>> 100);
    n = 4'bx01x;
    $display("shift x %b %b", u << n, s >>> n);
    n = 3;
    $display("shift var %b %b", u << n, s >>> n);
    w = u << 4; $display("ctx w %b", w);
    w = s >>> 4; $display("ctx w s %b", w);
    sw = s >>> 4; $display("ctx sw %b", sw);
    sw = s <<< 4; $display("ctx sw2 %b %d", sw, sw);
    w = {u << 4}; $display("concat %b", w);
    i = -16;
    $display("int >>> %0d >> %0d", i >>> 2, i >> 2);
    $display("int << %0d", i << 28);
    $display("lit %0d %0d", -8 >>> 1, -8 >> 1);
    $display("lit2 %h", 32'shffff0000 >>> 8);
    $display("lit3 %h", 32'hffff0000 >>> 8);
    $display("uns sgn %b", $unsigned(s) >>> 2);
    $display("sgn uns %b", $signed(u) >>> 2);
    $display("shift amt signed %b", u >> -1);
    $display("shift amt signed2 %b", u << 4'sb1111);
    $display("1<<31 %h %0d", 1 << 31, 1 << 31);
    $display("1<<32 %h", 1 << 32);
    $display("64 %h", 64'd1 << 63);
    $display("sext ctx %h", (s >>> 1) + 16'd0);
    $finish;
  end
endmodule
