// Writes to part of a memory element: `mem[a][7:0] <= d`, `mem[a][j]`,
// `mem[a][j +: 8]` and `mem[a][j -: 8]`, blocking and not, partly and wholly
// outside the element, with an `x` in the address, the index or an enable,
// on elements declared descending, ascending and signed, and the byte-write
// loop on a clock. Every line it prints is held to what xsim 2026.1 printed
// for it, `mem_part_tb.xsim` beside it.
`timescale 1ns / 1ps
module mem_part_tb;
  reg [31:0] m [0:3];
  reg [0:15] a [0:1];
  reg signed [7:0] s [0:1];
  reg [7:0] r [4:7];
  reg clk = 0;
  reg [3:0] we;
  reg [1:0] addr;
  reg [31:0] d;
  integer i, j;
  reg [4:0] k;
  reg signed [5:0] sk;

  task show(input [8*12-1:0] what);
    $display("%0s: m %h %h %h %h a %h %h s %h %h r %h %h", what, m[0], m[1], m[2], m[3],
             a[0], a[1], s[0], s[1], r[4], r[7]);
  endtask

  // The byte-write template: a loop over the lanes under their enables.
  always @(posedge clk)
    for (i = 0; i < 4; i = i + 1)
      if (we[i]) m[addr][i * 8 +: 8] <= d[i * 8 +: 8];

  initial begin
    show("start");
    m[0][7:0] = 8'h11;
    show("const");
    m[0][15:8] = 8'h22;
    m[0][31:24] = 8'h44;
    show("const2");
    m[0][16] = 1'b1;
    show("bit");
    k = 5'd20;
    m[0][k] = 1'b1;
    show("varbit");
    k = 5'bx;
    m[0][k] = 1'b0;
    show("xbit");
    m[1] = 0;
    j = 28;
    m[1][j +: 8] = 8'hff;
    show("up-high");
    m[2] = 0;
    j = 3;
    m[2][j -: 8] = 8'h5a;
    show("down-low");
    m[3] = 0;
    j = -2;
    m[3][j +: 4] = 4'hb;
    show("up-neg");
    j = 40;
    m[3][j +: 4] = 4'hf;
    show("up-out");
    sk = -6'sd1;
    m[3][sk +: 4] = 4'hc;
    show("signed");
    j = 'bx;
    m[3][j +: 4] = 4'h0;
    show("xbase");
    addr = 2'bx;
    m[addr][3:0] = 4'h5;
    show("xaddr");
    addr = 2'd1;
    m[addr][11:8] = 4'h7;
    show("varaddr");
    // An ascending element: bit 0 is its most significant.
    a[0] = 16'h0000;
    a[0][0:7] = 8'hab;
    show("asc");
    a[0][12 +: 4] = 4'hc;
    a[1] = 16'hffff;
    j = 4;
    a[1][j +: 4] = 4'h0;
    a[1][j -: 2] = 2'b01;
    show("asc2");
    // A signed element keeps its sign through a write of part of it.
    s[0] = 0;
    s[0][3:0] = 4'hf;
    s[0][7:4] = 4'h8;
    s[1] = -8'sd1;
    s[1][7] = 1'b0;
    show("signed2");
    $display("signed: %0d %0d", s[0], s[1]);
    // A memory not counted from zero.
    r[4] = 0;
    r[7] = 8'hff;
    r[4][3:0] = 4'h9;
    r[7][7:4] = 4'h0;
    show("offset");
    // Reads of part of an element. (One partly outside it is left out:
    // xsim reads the bits inside and `x` for the rest, where a read of
    // `n[j -: 8]` here reads all `x`, of an element as of any vector.)
    j = 4;
    $display("reads: %h %h %h %b", m[0][15:8], m[0][j +: 8], m[1][j + 7 -: 8], a[0][0:3]);
    // Byte writes on the clock, non-blocking: each lane enabled alone.
    m[0] = 32'h0;
    m[1] = 32'hffffffff;
    addr = 0;
    d = 32'hdeadbeef;
    we = 4'b0101;
    #1 clk = 1;
    #1 clk = 0;
    show("nba1");
    addr = 1;
    we = 4'b1010;
    d = 32'h01234567;
    #1 clk = 1;
    #1 clk = 0;
    show("nba2");
    // An unknown enable writes nothing; an unknown address nothing.
    we = 4'b1x0x;
    addr = 0;
    d = 32'h99999999;
    #1 clk = 1;
    #1 clk = 0;
    show("nba-xen");
    we = 4'b1111;
    addr = 2'bx1;
    #1 clk = 1;
    #1 clk = 0;
    show("nba-xaddr");
    // Two non-blocking writes to one word in one step: the later wins
    // where they overlap, and each keeps what it does not write.
    m[2] <= 32'h0;
    m[2][15:0] <= 16'h1111;
    m[2][11:4] <= 8'h22;
    m[2][31:28] <= 4'h3;
    #1 show("overlap");
    // A read in the same step sees the word before the write.
    m[3] = 32'h12345678;
    m[3][7:0] <= 8'hff;
    $display("before: %h", m[3]);
    #1 $display("after: %h", m[3]);
    $finish;
  end
endmodule
