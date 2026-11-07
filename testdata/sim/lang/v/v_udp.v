`timescale 1ns/1ps
primitive mux2 (out, s, a, b);
  output out;
  input s, a, b;
  table
  // s a b : out
     0 0 ? : 0;
     0 1 ? : 1;
     1 ? 0 : 0;
     1 ? 1 : 1;
     x 0 0 : 0;
     x 1 1 : 1;
  endtable
endprimitive
primitive dlatch (q, en, d);
  output q; reg q;
  input en, d;
  table
  // en d : q : q+
     1 0 : ? : 0;
     1 1 : ? : 1;
     0 ? : ? : -;
  endtable
endprimitive
primitive dff (q, clk, d);
  output q; reg q;
  input clk, d;
  initial q = 1'b0;
  table
  // clk d : q : q+
     (01) 0 : ? : 0;
     (01) 1 : ? : 1;
     (0?) ? : ? : -;
     (?0) ? : ? : -;
     (1x) ? : ? : -;
     ?   (??) : ? : -;
  endtable
endprimitive
primitive edge_p (q, clk, d);
  output q; reg q;
  input clk, d;
  table
     r 0 : ? : 0;
     r 1 : ? : 1;
     f ? : ? : -;
     ? * : ? : -;
  endtable
endprimitive
module tb;
  reg s, a, b, en, d, clk;
  wire m, l, f, e;
  mux2 u1 (m, s, a, b);
  dlatch u2 (l, en, d);
  dff u3 (f, clk, d);
  edge_p u4 (e, clk, d);
  integer i;
  reg [2:0] v;
  initial begin
    for (i = 0; i < 8; i = i + 1) begin
      {s, a, b} = i;
      #1 $display("mux s=%b a=%b b=%b -> %b", s, a, b, m);
    end
    s = 1'bx; a = 1; b = 1; #1 $display("mux sx 11 -> %b", m);
    s = 1'bx; a = 1; b = 0; #1 $display("mux sx 10 -> %b", m);
    s = 1'bz; a = 0; b = 0; #1 $display("mux sz 00 -> %b", m);
    $display("latch init %b dff init %b edge init %b", l, f, e);
    en = 1; d = 1; #1 $display("latch en d=1 %b", l);
    en = 0; d = 0; #1 $display("latch hold %b", l);
    clk = 0; d = 1; #1 clk = 1; #1 $display("dff posedge d=1 %b edge %b", f, e);
    d = 0; #1 $display("dff no edge %b edge %b", f, e);
    clk = 0; #1 $display("dff negedge %b edge %b", f, e);
    clk = 1; #1 $display("dff posedge d=0 %b edge %b", f, e);
    clk = 1'bx; d = 1; #1 clk = 1; #1 $display("dff x->1 %b edge %b", f, e);
    $finish;
  end
endmodule
