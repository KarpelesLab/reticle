`timescale 1ns/1ps
module tb;
  typedef logic [5:0] six_t;
  typedef enum logic [1:0] {C0, C1, C2} c_t;
  logic [7:0] a;
  logic [3:0] n;
  int i;
  c_t c;
  six_t s6;
  initial begin
    a = 8'hF3;
    $display("CA1 signed'(a)=%0d unsigned'(a)=%0d", signed'(a), unsigned'(a));
    $display("CA2 4'(a)=%h 12'(a)=%h", 4'(a), 12'(a));
    $display("CA3 12'(signed'(a))=%h", 12'(signed'(a)));
    s6 = six_t'(a);
    $display("CA4 six_t'(a)=%h", s6);
    $display("CA5 int'(a)=%0d int'(signed'(a))=%0d", int'(a), int'(signed'(a)));
    i = -1;
    $display("CA6 byte'(300)=%0d shortint'(i)=%0d", byte'(300), shortint'(i));
    c = c_t'(1);
    $display("CA7 c=%0d", c);
    c = c_t'(a[1:0]);
    $display("CA8 c=%0d", c);
    n = 4'(a + 8'h20);
    $display("CA9 n=%h", n);
    $display("CA10 sum of casts %0d", 4'(8'h1F) + 4'(8'h01));
    $display("CA11 bit'(x)=%b logic'(x)=%b", bit'(1'bx), logic'(1'bx));
    $display("CA12 2-state cast int'(4'bx1x0)=%0d", int'(4'bx1x0));
    i = 32'(5'b10011);
    $display("CA13 i=%0d", i);
    i = 32'(5'sb10011);
    $display("CA14 i=%0d", i);
    $display("CA15 real cast int'(2.5)=%0d int'(-2.5)=%0d int'(3.7)=%0d", int'(2.5), int'(-2.5), int'(3.7));
    $display("CA16 real'(7)/2=%f", real'(7) / 2);
    $finish;
  end
endmodule
