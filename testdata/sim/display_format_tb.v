// The `$display` conversions as xsim 2026.1 prints them; `display_format_tb.xsim`
// is its output for this file. Each line is one question about padding,
// leading zeros, unknown bits, signedness, strings or reals.
`timescale 1ns/1ps
module tb;
  reg [7:0] a; reg signed [7:0] s; reg [8*4:1] str; reg [15:0] w;
  reg signed [0:0] s1; reg signed [1:0] s2; reg signed [3:0] s4; reg signed [6:0] s7;
  reg signed [9:0] s10; reg signed [31:0] s32; reg signed [63:0] s64; reg signed [64:0] s65;
  real r;
  initial begin
    // Leading zeros: kept without a width, dropped by `%0`, zero-filled
    // up to an explicit width, space-filled after `-`.
    a = 8'd5; $display("A [%0h] [%0b] [%0o] [%h] [%b] [%o]", a, a, a, a, a, a);
    a = 8'h0f; $display("B [%3h] [%-4h] [%2o] [%5b] [%h] [%1h]", a, a, a, a, 12'h00f, a);
    a = 8'h0x; $display("C [%0h] [%0b] [%0o] [%4h] [%-4h] [%3o] [%5o]", a, a, a, a, a, a, a);
    a = 8'hx0; $display("D [%0h] [%0b] [%0o]", a, a, a);
    a = 8'hxx; $display("E [%4h] [%-4h] [%1h] [%3b] [%10b]", a, a, a, a, a);
    a = 8'b0000_00z1; $display("F [%0h] [%0b] [%0o]", a, a, a);
    a = 0; $display("G [%0h] [%0b] [%0o] [%0d] [%3h] [%1b]", a, a, a, a, a, a);
    // Upper-case conversions print lower case.
    w = 16'h00ab; $display("H [%X] [%H] [%0X] [%x] [%5x] [%05x]", 8'hab, 8'hcd, w, w, w, w);
    // Signed decimal fields: the digits of 2^N - 1 and one for the sign.
    s1 = -1; s2 = -1; s4 = -1; s7 = -1; s = -1; s10 = -1; s32 = -1; s64 = -1; s65 = -1;
    $display("I [%d] [%d] [%d] [%d] [%d] [%d] [%d] [%d] [%d]", s1, s2, s4, s7, s, s10, s32, s64, s65);
    s1 = 0; s4 = 3; s64 = 5;
    $display("J [%d] [%d] [%d] [%d] [%d]", s1, s4, s64, 4'sd3, 5'd3);
    // Unknown bits in decimal; a negative signed value with any is `x`.
    s = 8'b1000000z; a = 8'b1000000z; $display("K [%d] [%d] [%0d] [%5d] [%-5d]", s, a, s, s, s);
    s = 8'b0000000z; a = 8'b0000000z; $display("L [%d] [%d] [%5d] [%0d]", s, a, a, a);
    s = 8'bzzzzzzzz; $display("M [%d]", s);
    s = 8'b1000000x; $display("N [%d]", s);
    s = 8'bz0000001; $display("O [%d]", s);
    s = 8'bx0000000; $display("P [%d]", s);
    s = 8'b1xxxxxxx; $display("Q [%d] [%0d] [%d] [%0b]", s, s, 4'sb1x00, 4'sb1z00);
    // Explicit widths on known decimals, the zero flag.
    a = 8'h07; $display("R [%5d] [%-5d] [%1d] [%05d] [%5d] [%-5d]", a, a, a, a, 8'bx, 8'bx);
    s = -5; $display("S [%5d] [%-5d] [%1d] [%0d] [%h] [%5h]", s, s, s, s, s, s);
    // Strings: a NUL byte prints as a space, `%0s` drops the leading
    // ones, an unknown bit reads as zero, `-` does not left-justify.
    str = "ab"; $display("T [%s] [%5s] [%0s] [%-6s] [%1s]", str, str, str, str, str);
    str = 32'h41004200; $display("U [%s]", str);
    str = 0; $display("V [%s]", str);
    str = 32'h0041_0000; $display("W [%s]", str);
    $display("X [%s] [%s] [%s]", 8'bx, 16'h41xx, 8'hz0);
    $display("Y [%s] [%0s] [%5s] [%-5s]", "hi", "hi", "hi", "hi");
    // Reals: `-` is ignored; an integer conversion rounds to an integer.
    r = 3.14159;
    $display("Z [%-10.3f] [%10.3f] [%-10e] [%-10g] [%10d] [%0d] [%-12d] [%5f]", r, r, r, r, r, r, r, r);
    r = 1.0; $display("a [%d]", r);
    r = -2.5; $display("b [%d] [%0d] [%h] [%b]", r, r, r, r);
    r = 2.5; $display("c [%d] [%0d]", r, r);
    r = 123456789012.0; $display("d [%d]", r);
    r = 1.5; $display("e [%-8.2f|%8.2f] [%-8.2e] [%-8g] [%e] [%g] [%f]", r, r, r, r, r, r, r);
    r = 0.000012345; $display("f [%g] [%G] [%E] [%10.3g]", r, r, r, r);
    r = 1e20; $display("g [%g] [%f] [%d]", r, r, r);
    // Unformatted arguments, the library, the time.
    $display("h", 8'd5, " [%d]", 3);
    $display(8'd5, 4'shf, "|");
    $displayh(8'd5, " ", 16'd5);
    $displayb(4'd5, " ", 4'sd5);
    $displayo(8'd5);
    $display("i %l %L|");
    $display("j [%t] [%0t] [%h]", 10, 10, "A");
    // The zero flag fills on the left, before a sign too.
    $display("k [%05d] [%-05d] [%05s] [%08.2f] [%05h]", -5, -5, "ab", 1.5, 8'hab);
    a = 8'd5;
    $strobeh(a);
    $strobeb(a);
    $strobeo(a);
    $strobe(a);
    #1 $finish;
  end
endmodule
