`timescale 1ns/1ps
module tb;
  reg c;
  reg [3:0] a, b;
  reg [1:0] sel;
  reg signed [3:0] sa;
  reg [7:0] r;
  initial begin
    a = 4'b1010; b = 4'b1100;
    c = 1; $display("c1 %b", c ? a : b);
    c = 0; $display("c0 %b", c ? a : b);
    c = 1'bx; $display("cx %b", c ? a : b);
    c = 1'bz; $display("cz %b", c ? a : b);
    c = 1'bx; $display("cx same %b", c ? a : a);
    c = 1'bx; $display("cx xin %b", c ? 4'b1x0z : 4'b1100);
    sel = 2'b1x; $display("vec cond %b", sel ? a : b);
    sel = 2'b0x; $display("vec cond2 %b", sel ? a : b);
    sel = 2'b00; $display("vec cond3 %b", sel ? a : b);
    $display("nested %d", 1 ? (0 ? 1 : 2) : 3);
    $display("width %b", c ? 4'b1 : 8'hff);
    c = 0; r = c ? a : 8'hf0; $display("width2 %b", r);
    sa = -4'sd3;
    r = 1 ? sa : 4'sd0; $display("sign %b", r);
    r = 1 ? sa : 4'd0; $display("unsign %b", r);
    r = 1 ? sa : 8'sd0; $display("sign8 %b", r);
    c = 1'bx; r = c ? 8'd5 : 8'd7; $display("x merge %b", r);
    c = 1'bx; $display("x int %b", c ? 1 : 0);
    $display("real %f", c ? 1.5 : 1.5);
    c = 1; $display("real2 %f", c ? 1.5 : 2);
    c = 0; $display("real3 %f", c ? 1.5 : 2);
    $display("chain %0d", (a == 4'd10) ? 1 : (a == 4'd11) ? 2 : 3);
    $finish;
  end
endmodule
