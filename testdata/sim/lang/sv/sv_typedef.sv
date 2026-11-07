`timescale 1ns/1ps
typedef logic [7:0] byte_t;
typedef bit signed [11:0] s12_t;
typedef byte_t word_t [2];
typedef int unsigned u32;
module tb;
  typedef logic [3:0] nib_t;
  typedef nib_t [1:0] nib2_t;
  byte_t a;
  s12_t s;
  u32 u;
  nib_t n;
  nib2_t nn;
  word_t w;
  typedef enum {RED, GREEN} col_t;
  col_t c;
  initial begin
    a = 8'hA5;
    $display("T1 a=%h bits=%0d", a, $bits(a));
    s = -12'sd5;
    $display("T2 s=%0d %h bits=%0d", s, s, $bits(s));
    s = 12'h800;
    $display("T3 s=%0d", s);
    u = -1;
    $display("T4 u=%0d", u);
    n = 4'hF; n = n + 1;
    $display("T5 n=%0d", n);
    nn = 8'h3C;
    $display("T6 nn=%h nn[1]=%h nn[0]=%h bits=%0d", nn, nn[1], nn[0], $bits(nn));
    w[0] = 8'h11; w[1] = 8'h22;
    $display("T7 w0=%h w1=%h bits=%0d", w[0], w[1], $bits(w));
    c = GREEN;
    $display("T8 c=%0d %s", c, c.name());
    $display("T9 bits(byte_t)=%0d bits(s12_t)=%0d bits(nib2_t)=%0d", $bits(byte_t), $bits(s12_t), $bits(nib2_t));
    s = 12'sd100; a = 8'd3;
    $display("T10 s*a=%0d", s * $signed(a));
    $finish;
  end
endmodule
