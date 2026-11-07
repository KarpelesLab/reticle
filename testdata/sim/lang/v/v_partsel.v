`timescale 1ns/1ps
module tb;
  reg [15:0] d;
  reg [0:15] asc;
  reg [23:8] nz;
  reg [7:0] r;
  integer i;
  initial begin
    d = 16'hA5C3; asc = 16'hA5C3; nz = 16'hA5C3;
    $display("d[7:0] %h d[15:8] %h d[3] %b", d[7:0], d[15:8], d[3]);
    $display("asc[0:7] %h asc[8:15] %h asc[3] %b", asc[0:7], asc[8:15], asc[3]);
    $display("nz[15:8] %h nz[23:16] %h nz[8] %b", nz[15:8], nz[23:16], nz[8]);
    $display("+: %h %h", d[0 +: 8], d[4 +: 8]);
    $display("-: %h %h", d[15 -: 8], d[11 -: 8]);
    $display("asc +: %h -: %h", asc[0 +: 8], asc[15 -: 8]);
    $display("nz +: %h -: %h", nz[8 +: 8], nz[23 -: 4]);
    for (i = 0; i < 16; i = i + 4) $display("var +: i=%0d %h", i, d[i +: 4]);
    for (i = 15; i > 0; i = i - 4) $display("var -: i=%0d %h", i, d[i -: 4]);
    i = 14; $display("partial oob +: %b", d[i +: 4]);
    i = -2; $display("partial oob low %b", d[i +: 4]);
    i = 20; $display("whole oob %b", d[i +: 4]);
    i = 'bx; $display("x index %b %b", d[i +: 4], d[i]);
    $display("bit oob %b %b", d[16], d[-1]);
    i = 16; $display("bit oob var %b", d[i]);
    d = 16'h0000;
    i = 14; d[i +: 4] = 4'hf; $display("write partial oob %h", d);
    d = 16'h0000;
    i = 20; d[i +: 4] = 4'hf; $display("write oob %h", d);
    i = 16; d[i] = 1; $display("write bit oob %h", d);
    i = 'bx; d[i] = 1; $display("write bit x %h", d);
    d[i +: 4] = 4'hf; $display("write part x %h", d);
    d = 0; d[7:4] = 4'hb; $display("write const %h", d);
    d = 0; d[3 +: 2] = 2'b11; $display("write +: %b", d);
    d = 0; d[3 -: 2] = 2'b11; $display("write -: %b", d);
    asc = 0; asc[0 +: 4] = 4'b1000; $display("asc write %b", asc);
    nz = 0; nz[20 -: 4] = 4'b1001; $display("nz write %b", nz);
    d = 16'h1234;
    r = d[11:4]; $display("r %h", r);
    r = d[3:0]; $display("r ext %h", r);
    r = d[i]; $display("r x %b", r);
    d = 16'hFFFF; r = d[15 -: 8] + 1; $display("sel unsigned add %h", r);
    $finish;
  end
endmodule
