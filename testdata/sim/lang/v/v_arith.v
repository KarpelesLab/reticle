`timescale 1ns/1ps
module tb;
  reg [7:0] a, b;
  reg signed [7:0] sa, sb;
  integer i, j;
  reg [15:0] w;
  initial begin
    a = 8'd200; b = 8'd100;
    w = a + b;  $display("add16 %d %h", w, w);
    $display("add8 %d", a + b);
    w = a - b;  $display("sub %d", w);
    w = b - a;  $display("subneg %d %h %b", w, w, w);
    w = a * b;  $display("mul %d", w);
    $display("div %d mod %d", a / b, a % 8'd7);
    sa = -8'sd7; sb = 8'sd2;
    $display("sdiv %d smod %d", sa / sb, sa % sb);
    sa = 8'sd7; sb = -8'sd2;
    $display("sdiv2 %d smod2 %d", sa / sb, sa % sb);
    sa = -8'sd7; sb = -8'sd2;
    $display("sdiv3 %d smod3 %d", sa / sb, sa % sb);
    i = -7; j = 2;
    $display("idiv %0d imod %0d", i / j, i % j);
    $display("idiv2 %0d imod2 %0d", 7 / -2, 7 % -2);
    $display("divzero %d %h %b", a / 8'd0, a / 8'd0, a % 8'd0);
    $display("idivzero %d", i / 0);
    $display("idivzero2 %d", i % 0);
    $display("pow %d %d %d", 2**10, 3**4, 2**0);
    $display("pow neg %0d %0d %0d", (-2)**3, (-1)**5, 2**-1);
    $display("pow0 %0d %0d", 0**0, 0**-1);
    i = -3;
    $display("pow int %0d %0d", i**2, i**3);
    $display("pow u %d", 8'd3 ** 8'd5);
    $display("pow sz %h", 4'd2 ** 3'd3);
    $display("unary %d %d", -a, -sa);
    $display("unary2 %h %b", -8'd1, ~8'd5);
    $display("plus %d", +sa);
    $display("ovf %d", 8'd255 + 8'd1);
    w = 8'd255 + 8'd1; $display("ovf ctx %d", w);
    i = 32'h7fffffff; i = i + 1; $display("intovf %0d %h", i, i);
    i = -32'sd2147483647 - 1; $display("intmin %0d", i);
    $display("mul neg %0d", -5 * 3);
    $display("big %h", 64'hffffffff_ffffffff + 1);
    $display("big2 %h", 33'h1_ffffffff * 2);
    $display("sub unsized %0d", 3 - 5);
    $display("sub sized %0d", 4'd3 - 4'd5);
    $display("mixed %0d", 4'd3 - 5);
    $finish;
  end
endmodule
