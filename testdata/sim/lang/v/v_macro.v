`timescale 1ns/1ps
`define W 8
`define MAX(a,b) ((a) > (b) ? (a) : (b))
`define SQ(x) ((x)*(x))
`define STR "macro string"
`define EMPTY
`define MULTI(a) \
  $display("multi line %0d", a); \
  $display("second line %0d", a + 1)
`define FEATURE
`ifdef FEATURE
  `define MODE 1
`else
  `define MODE 0
`endif
`ifndef UNDEFINED_THING
  `define NOTDEF 42
`endif
module tb;
  reg [`W-1:0] r;
  integer i;
  initial begin
    r = `W'hff;
    $display("W=%0d r=%h", `W, r);
    $display("MAX %0d %0d", `MAX(3, 7), `MAX(10, -2));
    $display("SQ %0d %0d", `SQ(4), `SQ(1+2));
    $display("STR %s", `STR);
    `MULTI(5);
    $display("MODE %0d NOTDEF %0d", `MODE, `NOTDEF);
`ifdef UNDEFINED_THING
    $display("should not print");
`elsif FEATURE
    $display("elsif branch");
`else
    $display("else branch");
`endif
`undef FEATURE
`ifdef FEATURE
    $display("undef failed");
`else
    $display("undef worked");
`endif
    $display("empty`EMPTY macro");
    $finish;
  end
endmodule
