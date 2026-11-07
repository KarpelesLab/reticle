`timescale 1ns/1ps
module tb;
  reg [7:0] a, b;
  reg [3:0] x;
  reg [7:0] c;
  initial begin
    a = 8'b1100_1010; b = 8'b1010_0110;
    $display("and %b or %b xor %b", a & b, a | b, a ^ b);
    $display("xnor %b xnor2 %b", a ~^ b, a ^~ b);
    $display("not %b", ~a);
    $display("red & %b | %b ^ %b", &a, |a, ^a);
    $display("red ~& %b ~| %b ~^ %b", ~&a, ~|a, ~^a);
    $display("red ones %b %b", &8'hff, ~|8'h00);
    x = 4'b10xz;
    $display("x and %b or %b xor %b", x & 4'b1111, x | 4'b0000, x ^ 4'b0101);
    $display("x and0 %b or1 %b", x & 4'b0000, x | 4'b1111);
    $display("x not %b", ~x);
    $display("x red & %b | %b ^ %b", &x, |x, ^x);
    $display("x red2 %b %b", &4'b0x1z, |4'b1xz0);
    $display("log && %b || %b ! %b", a && b, a || 0, !a);
    $display("log0 %b %b", !8'd0, 8'd0 && 1'bx);
    $display("logx %b %b %b", 1'bx && 1'b1, 1'bx || 1'b1, !1'bx);
    $display("logx2 %b %b", x && 1, 4'b00x0 || 0);
    $display("logz %b", 4'b000z && 1);
    $display("width mix %b", 4'b1010 & 8'b11111111);
    c = 4'b1010 | 8'b11110000; $display("ctx %b", c);
    c = ~4'b1010; $display("ctx not %b", c);
    $display("ctx not2 %b", ~4'b1010);
    $display("eq %b neq %b", a == b, a != b);
    $display("lt %b %b %b %b", a < b, a <= b, a > b, a >= b);
    $display("x cmp %b %b", x == 4'b10xz, x === 4'b10xz);
    $display("x cmp2 %b %b", x != 4'b0000, x !== 4'b10x0);
    $display("x cmp3 %b %b", x == 4'b0000, x < 4'b1111);
    $display("hex x %h %h", x, 8'bxxxx_0101);
    $display("hex z %h %h", 8'bzzzz_zzzz, 8'b1zzz_0000);
    $display("oct %o %o", 9'b101_xxx_zzz, 6'o77);
    $display("dec x %d %d", 8'bxxxx_xxxx, 8'bzzzz_zzzz);
    $display("dec partx %d %d", 8'b0000_x101, 8'bzzzz_0101);
    $finish;
  end
endmodule
