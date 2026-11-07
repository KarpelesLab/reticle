`timescale 1ns/1ps
module tb;
  reg signed [3:0] s4;
  reg signed [7:0] s8;
  reg signed [15:0] s16;
  reg signed [31:0] s32;
  reg signed [63:0] s64;
  reg signed [4:0] s5;
  reg signed [6:0] s7;
  reg signed [9:0] s10;
  integer i;
  initial begin
    s4 = -8; s8 = -128; s16 = -32768; s32 = -1; s64 = -64'sd9223372036854775807 - 1;
    s5 = -16; s7 = -64; s10 = -512;
    $display("[%d][%d][%d][%d]", s4, s8, s16, s32);
    $display("[%d]", s64);
    $display("[%d][%d][%d]", s5, s7, s10);
    $display("[%0d][%0d][%0d][%0d]", s4, s8, s16, s32);
    s4 = 7; s8 = 127; s16 = 32767; s32 = 2147483647;
    $display("[%d][%d][%d][%d]", s4, s8, s16, s32);
    s4 = 0; s8 = 1;
    $display("[%d][%d]", s4, s8);
    $display("hex [%h][%h][%h]", -4'sd1, s16, -8'sd2);
    $display("bin [%b] oct [%o]", -4'sd3, -8'sd1);
    $display("expr [%d] [%d]", s8 - 2, -s8);
    $display("unsigned of neg [%d]", $unsigned(-8'sd1));
    $display("signed of u [%d]", $signed(8'hff));
    s8 = 8'bx0000000; $display("x signed [%d]", s8);
    s8 = 8'b1000000z; $display("z signed [%d]", s8);
    i = -2147483648; $display("intmin [%d] [%0d]", i, i);
    $display("lit [%d] [%d]", -5, 5);
    $display("lit sized [%d] [%d]", -8'sd5, 8'sd5);
    $display("unsized neg [%d]", -'sd5);
    $display(s8);
    $display(-8'sd5);
    $display(i);
    $display(-1);
    $finish;
  end
endmodule
