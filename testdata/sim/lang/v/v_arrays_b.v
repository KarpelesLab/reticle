`timescale 1ns/1ps
module tb;
  integer ia [0:3];
  time ta [1:2];
  reg signed [15:0] sa [3:0];
  reg [0:7] asc [0:1];
  wire [7:0] wa [0:1];
  reg [7:0] src;
  integer i;
  assign wa[0] = src;
  assign wa[1] = ~src;
  initial begin
    for (i = 0; i < 4; i = i + 1) ia[i] = (i - 2) * 1000;
    for (i = 0; i < 4; i = i + 1) $display("ia[%0d]=%0d", i, ia[i]);
    ta[1] = 64'd123456789012; ta[2] = ta[1] * 2;
    $display("ta %0d %0d", ta[1], ta[2]);
    sa[0] = -16'sd300; sa[3] = 16'sd300;
    $display("sa %0d %0d sum %0d", sa[0], sa[3], sa[0] + sa[3]);
    $display("sa cmp %b", sa[0] < sa[3]);
    asc[0] = 8'b1000_0001; asc[1] = 8'hF0;
    $display("asc %b bit0 %b bit7 %b part %b", asc[0], asc[0][0], asc[0][7], asc[1][0:3]);
    src = 8'h3c;
    #1 $display("wa %h %h", wa[0], wa[1]);
    i = 2; $display("ia var %0d", ia[i]);
    i = 5; $display("ia oob %0d", ia[i]);
    ia[1] = ia[1] + ia[2];
    $display("ia rmw %0d", ia[1]);
    $finish;
  end
endmodule
