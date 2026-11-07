`timescale 1ns/1ps
module tb;
  reg [7:0] m [0:7];
  reg [7:0] r [7:0];
  reg signed [7:0] sm [0:3];
  reg mb [0:15];
  integer i, j, k;
  reg [7:0] t;
  initial begin
    for (i = 0; i < 8; i = i + 1) begin m[i] = i * 3; r[i] = 8'h80 | i; end
    for (i = 0; i < 8; i = i + 1) $display("m[%0d]=%h r[%0d]=%h", i, m[i], i, r[i]);
    $display("oob read %h %h", m[8], m[-1]);
    i = 8; $display("oob var read %h", m[i]);
    i = 'bx; $display("x addr read %h", m[i]);
    i = 8; m[i] = 8'hff; $display("oob write ignored %h %h", m[7], m[0]);
    $display("bit of word %b %h", m[3][0], m[5][7:4]);
    i = 6; $display("var bit %b %h", m[i][1], m[i][3 +: 4]);
    m[2][7] = 1; $display("write bit %h", m[2]);
    m[3][3:0] = 4'ha; $display("write part %h", m[3]);
    for (i = 0; i < 4; i = i + 1)
    for (i = 1; i < 3; i = i + 1)
      for (j = 0; j < 2; j = j + 1)
    sm[0] = -8'sd5; sm[1] = 8'sd5;
    $display("signed mem %d %d", sm[0], sm[1]);
    $display("signed mem cmp %b", sm[0] < sm[1]);
    $display("signed mem add %0d", sm[0] + 1);
    for (i = 0; i < 16; i = i + 1) mb[i] = i[0];
    $display("mb %b%b%b%b", mb[0], mb[1], mb[2], mb[3]);
    $display("uninit %h", sm[3]);
    t = m[1] + m[2]; $display("sum %h", t);
    m[0] = m[1]; $display("copy %h", m[0]);
    $finish;
  end
endmodule
