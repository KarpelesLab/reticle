`timescale 1ns/1ps
module tb;
  reg [8*8:1] s8;
  reg [8*3:1] s3;
  reg [7:0] ch;
  reg [8*12:1] msg;
  integer i;
  initial begin
    s8 = "Verilog"; $display("[%s] [%h]", s8, s8);
    s3 = "toolong"; $display("trunc [%s] [%h]", s3, s3);
    s8 = ""; $display("empty [%s] [%h]", s8, s8);
    s8 = "ab"; $display("pad [%s] [%h]", s8, s8);
    ch = "A"; $display("char %c %d %h", ch, ch, ch);
    ch = "A" + 1; $display("char+1 %c", ch);
    s8 = "abc";
    for (i = 0; i < 3; i = i + 1) $display("byte %0d = %c", i, s8[8*i+1 +: 8]);
    s8 = {"ab", "cd"}; $display("cat [%s]", s8);
    s8 = {2{"xy"}}; $display("rep [%s]", s8);
    $display("cmp %b %b", "abc" == "abc", "abc" == "abd");
    $display("lit %h", "AB");
    $display("lit d %0d", "A");
    msg = "tab\tend"; $display("esc [%s]", msg);
    msg = "q\"x\\y"; $display("esc2 [%s]", msg);
    msg = "\101\102\103"; $display("octal [%s]", msg);
    s8 = "hello"; $display("as s with nulls [%s]", s8);
    s8 = 64'h0041_4200_4300_0000; $display("embedded nul [%s]", s8);
    $display("s of number [%s]", 32'h41424344);
    $display("s of x [%s]", 8'bx);
    $display("%s and %s", "one", "two");
    $finish;
  end
endmodule
