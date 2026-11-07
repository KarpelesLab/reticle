`timescale 1ns/1ps
module tb;
  logic [11:0] v;
  logic signed [7:0] s;
  bit [7:0] b;
  logic [3:0][3:0] p;
  int i;
  initial begin
    v = '0;
    $display("FL1 '0=%b", v);
    v = '1;
    $display("FL2 '1=%b %h", v, v);
    v = 'x;
    $display("FL3 'x=%b", v);
    v = 'z;
    $display("FL4 'z=%b", v);
    s = '1;
    $display("FL5 signed '1=%0d", s);
    b = 'x;
    $display("FL6 bit 'x=%b", b);
    p = '1;
    $display("FL7 packed md '1=%h", p);
    i = '1;
    $display("FL8 int '1=%0d", i);
    v = 12'hABC;
    v = ~'0;
    $display("FL9 ~'0 v=%h", v);
    v = 12'h0 | '1;
    $display("FL10 expr '1 v=%h", v);
    $display("FL11 '1 in compare %b", v == '1);
    v = 12'b0;
    v[7:4] = '1;
    $display("FL12 partial '1 %b", v);
    $display("FL13 '1+1 in 12bit ctx %h", 12'('1 + 1'b1));
    $finish;
  end
endmodule
