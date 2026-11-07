// `%p` as xsim 2026.1 prints it (`pattern_tb.xsim`): a struct with its
// member names (without under `%0p`), an enum by name or, for a value it
// does not name, by number, an array element by element, a known vector
// in decimal, one with an unknown bit or a single bit sized in binary, a
// real as `%g`; a width pads, and `-` does not left-justify.
`timescale 1ns/1ps
module tb;
  typedef enum logic [1:0] {RED, GRN, BLU} col_t;
  typedef struct packed { col_t c; logic [3:0] n; } in_t;
  typedef struct packed { in_t i; logic s; logic signed [3:0] k; } out_t;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } ps_t;
  ps_t p;
  col_t col, bad;
  out_t o;
  logic [7:0] v, q;
  logic [7:0] la [2];
  col_t ca [2];
  int arr [3];
  logic signed [7:0] sv;
  logic one, unk;
  bit bb;
  bit [3:0] b4;
  int i;
  byte by;
  real r, big;
  initial begin
    p = 8'h3F; v = 8'h3F; col = GRN; bad = col_t'(2'd3);
    o = {BLU, 4'hz, 1'b1, 4'b1110};
    la[0] = 8'h1x; la[1] = 8'd7;
    ca[0] = GRN; ca[1] = col_t'(2'd3);
    arr[0] = 1; arr[1] = 2; arr[2] = 255;
    sv = 8'sb1x00_0000; one = 1; unk = 1'bx; bb = 0; b4 = 4'd9; q = 8'hff;
    i = -5; by = -3; r = 3.14159; big = 1e20;
    $display("A [%p] [%p] [%0p]", p, v, p);
    $display("B [%p] [%p] [%0p]", col, bad, bad);
    $display("C [%p]", o);
    $display("D [%0p]", o);
    $display("E [%p] [%0p] [%p] [%p]", la, la, ca, arr);
    $display("F [%p] [%p] [%p] [%p] [%p] [%5p] [%-5p]|", sv, one, unk, bb, b4, q, q);
    $display("G [%p] [%p] [%p] [%p]", i, by, r, big);
    $display("H [%p] [%0p]", o.i, o.k);
    $finish;
  end
endmodule
