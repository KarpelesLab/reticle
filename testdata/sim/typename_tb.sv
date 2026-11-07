// `$typename` as xsim 2026.1 spells it (`typename_tb.xsim`). IEEE
// 1800-2017 §20.6.1 leaves the spelling to the tool, so each line is
// xsim's choice: no space before a packed range, `$` before an unpacked
// one, an enum or struct named with the scope of its typedef, any other
// typedef not named at all.
`timescale 1ns/1ps
package pk;
  typedef enum bit [2:0] {A, B} pe_t;
endpackage
module tb;
  int i;
  int unsigned iu;
  integer ig;
  integer unsigned iun;
  byte by;
  byte unsigned bu;
  shortint si;
  longint li;
  time t;
  real r;
  string s;
  bit b1;
  bit [3:0] bt;
  reg [2:0] rg;
  logic sl;
  logic [3:0] l;
  logic signed [3:0] ls;
  logic [3:0][1:0] md;
  wire [7:0] w;
  logic [7:0] la [0:3];
  int arr [3];
  pk::pe_t pe;
  typedef logic [5:0] six_t;
  six_t six;
  typedef enum {X, Y} ie_t;
  ie_t ie;
  typedef enum logic [1:0] {RED, GRN, BLU} col_t;
  col_t col;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } ps_t;
  ps_t p;
  logic [7:0] a8, b8;
  initial begin
    $display("A %s|%s|%s|%s|%s|%s", $typename(i), $typename(iu), $typename(ig), $typename(iun), $typename(by), $typename(bu));
    $display("B %s|%s|%s|%s|%s", $typename(si), $typename(li), $typename(t), $typename(r), $typename(s));
    $display("C %s|%s|%s|%s|%s|%s", $typename(b1), $typename(bt), $typename(rg), $typename(sl), $typename(l), $typename(ls));
    $display("D %s|%s|%s|%s", $typename(md), $typename(w), $typename(la), $typename(arr));
    $display("E %s|%s|%s|%s|%s", $typename(pe), $typename(six), $typename(ie), $typename(col), $typename(p));
    $display("F %s|%s", $typename(a8 + b8), $typename(int));
    $finish;
  end
endmodule
