`timescale 1ns/1ps
module tb;
  reg [7:0] u8;
  reg [3:0] u4;
  reg signed [7:0] s8;
  reg signed [3:0] s4;
  integer i;
  initial begin
    u8 = 8'd200; u4 = 4'd15; s8 = -8'sd1; s4 = -4'sd1;
    $display("u8>u4 %b u8==u4 %b", u8 > u4, u8 == u4);
    $display("s8==s4 %b (both -1 signed)", s8 == s4);
    $display("s8==u4 %b (mixed)", s8 == u4);
    $display("s4==8'hff %b", s4 == 8'hff);
    $display("s4==8'shff %b", s4 == 8'shff);
    $display("s4<s8 %b s4<=s8 %b", s4 < s8, s4 <= s8);
    $display("s8<0 %b s8<1'b0 %b s8<1'sb0 %b", s8 < 0, s8 < 1'b0, s8 < 1'sb0);
    $display("s8>u8 %b", s8 > u8);
    i = -1;
    $display("i<u8 %b i<$signed(u8) %b", i < u8, i < $signed(u8));
    $display("i==-1 %b i==32'hffffffff %b", i == -1, i == 32'hffffffff);
    $display("4'b1x==4'b1x %b ===%b", 4'b101x == 4'b101x, 4'b101x === 4'b101x);
    $display("x< %b x>= %b", 4'b1x00 < 4'b1111, 4'b0x00 >= 4'b0000);
    $display("z== %b z=== %b", 1'bz == 1'bz, 1'bz === 1'bz);
    $display("x!==z %b", 1'bx !== 1'bz);
    $display("width ext == %b %b", 4'b1010 == 8'b00001010, 4'b1010 === 8'bx0001010);
    $display("cmp result width %b", (u8 > u4) + 1'b1);
    $display("cmp in arith %0d", (u8 > u4) * 10 + (u8 < u4));
    $display("chain %b", 3 > 2 > 1);
    $display("logic eq %b %b", (u8 && u4) == 1, !u8 == 0);
    $display("signed lit cmp %b %b", -4'sd8 < 4'sd7, 4'sd8 < 4'sd7);
    $display("unsigned lit cmp %b", -4'd8 < 4'd7);
    $display("real cmp %b %b", 1.5 > 1, 2 == 2.0);
    $display("string cmp %b %b", "abc" < "abd", "b" > "abc");
    $finish;
  end
endmodule
