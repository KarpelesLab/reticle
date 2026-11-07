// The SystemVerilog bit-counting functions on run-time values, as xsim
// 2026.1 computes them (`bitcount_tb.xsim`). Every operand is a variable,
// so nothing folds at elaboration.
`timescale 1ns/1ps
module tb;
  logic [7:0] v = 8'b1011_0010;
  logic [7:0] u = 8'b01x0_z001;
  logic [7:0] oh = 8'b0001_0000;
  logic [7:0] z = 0;
  logic [7:0] ox = 8'b0000_000x;
  logic [3:0] q = 4'b1x1z;
  logic [69:0] wide = {1'b1, 68'b0, 1'b1};
  initial begin
    $display("A %0d %0d %0d %0d", $countones(v), $countones(u), $countones(z), $countones(wide));
    $display("B %b %b %b %b %b", $onehot(v), $onehot(oh), $onehot(z), $onehot(u), $onehot(ox));
    $display("C %b %b %b %b", $onehot0(z), $onehot0(oh), $onehot0(v), $onehot0(ox));
    $display("D %b %b %b %b", $isunknown(u), $isunknown(v), $isunknown(ox), $isunknown(q));
    $display("E %0d %0d %0d %0d %0d", $countbits(u, '1), $countbits(u, '0), $countbits(u, 'x), $countbits(u, 'z), $countbits(u, '1, 'x));
    $display("F %0d %0d", $countbits(q, 1'b1, 1'bz), $countbits(v, '0, '1));
    $display("G %0d %b", $countones(v) + 1, $onehot(v) | 1'b1);
    $finish;
  end
endmodule
