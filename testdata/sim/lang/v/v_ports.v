`timescale 1ns/1ps
module add4 (input [3:0] a, input [3:0] b, output [4:0] s);
  assign s = a + b;
endmodule
module sgn (input signed [3:0] a, output signed [7:0] y);
  assign y = a;
endmodule
module oldstyle (a, b, y);
  input [7:0] a;
  input b;
  output [7:0] y;
  reg [7:0] y;
  always @* y = b ? a : ~a;
endmodule
module withdef (input [3:0] a, input [3:0] b, output [3:0] y, output [3:0] z);
  assign y = a | b;
  assign z = a & b;
endmodule
module inoutm (inout [3:0] p, input en, input [3:0] v);
  assign p = en ? v : 4'bz;
endmodule
module tb;
  reg [3:0] a, b;
  reg [7:0] wa;
  wire [4:0] s1, s2;
  wire [2:0] s_narrow;
  wire [7:0] s_wide;
  wire signed [7:0] y1, y2;
  wire [7:0] o1;
  wire [3:0] y3, z3, y4;
  wire [3:0] bus;
  reg en1, en2;
  add4 u1 (a, b, s1);
  add4 u2 (.b(b), .a(a), .s(s2));
  add4 u3 (.a(wa), .b(4'd1), .s(s_narrow));
  add4 u4 (.a(a), .b(b), .s(s_wide));
  add4 u5 (.a(2'b11), .b(), .s());
  sgn  u6 (.a(a), .y(y1));
  sgn  u7 (.a(4'sb1000), .y(y2));
  oldstyle u8 (.a(wa), .b(a[0]), .y(o1));
  withdef u9 (.a(a), .b(b), .y(y3), .z());
  withdef u10 (a, b, y4, );
  inoutm u11 (.p(bus), .en(en1), .v(4'h5));
  add4 ua [1:0] (.a({a, b}), .b(4'd1), .s());
  initial begin
    a = 4'd9; b = 4'd8; wa = 8'hf3; en1 = 0;
    #1 $display("s1=%0d s2=%0d narrow=%b wide=%h", s1, s2, s_narrow, s_wide);
    $display("y1=%b y2=%b %0d", y1, y2, y2);
    $display("o1=%h y3=%b y4=%b", o1, y3, y4);
    $display("u5.s=%b u5.b=%b", u5.s, u5.b);
    $display("bus off %b", bus);
    en1 = 1;
    #1 $display("bus on %b", bus);
    $display("hier u1.a=%0d u2.s=%0d u8.y=%h", u1.a, u2.s, u8.y);
    $display("array ua[0].s=%0d ua[1].s=%0d", ua[0].s, ua[1].s);
    $finish;
  end
endmodule
