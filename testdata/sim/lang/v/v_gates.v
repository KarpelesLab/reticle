`timescale 1ns/1ps
module tb;
  reg a, b, c, en;
  wire o_and, o_or, o_nand, o_nor, o_xor, o_xnor, o_buf, o_not, o_and3;
  wire o_b0, o_b1, o_n0, o_n1;
  wire o_buf2a, o_buf2b;
  wire [1:0] va;
  wire o_dly;
  and  g1 (o_and, a, b);
  or   g2 (o_or, a, b);
  nand g3 (o_nand, a, b);
  nor  g4 (o_nor, a, b);
  xor  g5 (o_xor, a, b);
  xnor g6 (o_xnor, a, b);
  buf  g7 (o_buf, a);
  not  g8 (o_not, a);
  and  g9 (o_and3, a, b, c);
  bufif0 g10 (o_b0, a, en);
  bufif1 g11 (o_b1, a, en);
  notif0 g12 (o_n0, a, en);
  notif1 g13 (o_n1, a, en);
  buf g14 (o_buf2a, o_buf2b, a);
  and ga [1:0] (va, {a, b}, {c, c});
  and #2 g15 (o_dly, a, b);
  nmos (nm, a, en);
  pmos (pm, a, en);
  integer i;
  reg [3:0] vals;
  initial begin
    vals = 4'b01xz;
    for (i = 0; i < 16; i = i + 1) begin
      a = vals[i / 4]; b = vals[i % 4]; c = 1; en = 1;
      #1 $display("a=%b b=%b and=%b or=%b nand=%b nor=%b xor=%b xnor=%b buf=%b not=%b and3=%b",
        a, b, o_and, o_or, o_nand, o_nor, o_xor, o_xnor, o_buf, o_not, o_and3);
    end
    for (i = 0; i < 16; i = i + 1) begin
      a = vals[i / 4]; en = vals[i % 4];
      #1 $display("a=%b en=%b bufif0=%b bufif1=%b notif0=%b notif1=%b nmos=%b pmos=%b",
        a, en, o_b0, o_b1, o_n0, o_n1, nm, pm);
    end
    a = 1; b = 0; c = 1;
    #1 $display("multi-out buf %b %b array %b", o_buf2a, o_buf2b, va);
    a = 1; b = 1;
    #1 $display("%0t dly %b", $time, o_dly);
    #2 $display("%0t dly %b", $time, o_dly);
    $finish;
  end
endmodule
