`timescale 1ns/1ps
module tb;
  logic [15:0] w;
  logic [0:15] bw;
  logic [7:0] v;
  int i;
  initial begin
    w = 16'hA5C3;
    bw = 16'hA5C3;
    $display("BS1 w[3:0]=%h w[15:12]=%h bw[0:3]=%h bw[12:15]=%h", w[3:0], w[15:12], bw[0:3], bw[12:15]);
    i = 4;
    $display("BS2 w[i+:4]=%h w[i-:4]=%h bw[i+:4]=%h bw[i-:4]=%h", w[i +: 4], w[i -: 4], bw[i +: 4], bw[i -: 4]);
    i = 14;
    $display("BS3 part oob w[i+:4]=%b", w[i +: 4]);
    i = -1;
    $display("BS4 neg idx w[i]=%b", w[i]);
    i = 'x;
    $display("BS5 x idx w[i]=%b", w[i]);
    w[i] = 1'b0;
    $display("BS6 write x idx w=%h", w);
    i = 8;
    w[i +: 4] = 4'h0;
    $display("BS7 w=%h", w);
    w[i -: 2] = 2'b11;
    $display("BS8 w=%h", w);
    v = 8'hFF;
    v[i] = 1'b0;
    $display("BS9 oob write v=%h", v);
    $finish;
  end
endmodule
