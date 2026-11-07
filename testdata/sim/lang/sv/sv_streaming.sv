`timescale 1ns/1ps
module tb;
  logic [7:0] a, r8;
  logic [15:0] w, r16;
  logic [31:0] d, r32;
  logic [3:0] n;
  logic [7:0] x, y;
  initial begin
    a = 8'b1100_1010;
    r8 = {<<{a}};
    $display("SM1 {<<{a}}=%b", r8);
    r8 = {>>{a}};
    $display("SM2 {>>{a}}=%b", r8);
    w = 16'hABCD;
    r16 = {<<4{w}};
    $display("SM3 {<<4{w}}=%h", r16);
    r16 = {<<8{w}};
    $display("SM4 {<<8{w}}=%h", r16);
    r16 = {>>8{w}};
    $display("SM5 {>>8{w}}=%h", r16);
    d = 32'h1234_5678;
    r32 = {<<byte{d}};
    $display("SM6 {<<byte{d}}=%h", r32);
    r32 = {<<16{d}};
    $display("SM7 {<<16{d}}=%h", r32);
    r8 = {<<2{a}};
    $display("SM8 {<<2{a}}=%b", r8);
    r8 = {<<3{a}};
    $display("SM9 {<<3{a}}=%b", r8);
    w = {<<{a, 8'h01}};
    $display("SM10 concat stream=%h", w);
    {<<{n}} = 4'b0001;
    $display("SM11 unpack lhs n=%b", n);
    {>>{x, y}} = 16'hBEEF;
    $display("SM12 lhs split x=%h y=%h", x, y);
    {<<8{x, y}} = 16'h1234;
    $display("SM13 lhs reverse x=%h y=%h", x, y);
    d = {<<{8'h01}};
    $display("SM14 widened d=%h", d);
    $display("SM15 cast in display %b", 8'({<<{a}}));
    $finish;
  end
endmodule
