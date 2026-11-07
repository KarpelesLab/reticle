`timescale 1ns/1ps
module tb;
  integer i, j;
  reg [31:0] u;
  reg [63:0] w;
  time t;
  reg [7:0] b;
  initial begin
    i = 2147483647; i = i + 1; $display("ovf %0d", i);
    i = -2147483648; i = i - 1; $display("unf %0d", i);
    i = 65536; j = 65536; $display("mul ovf %0d", i * j);
    w = i * j; $display("mul ovf ctx %0d", w);
    w = 64'd65536 * j; $display("mul wide %0d", w);
    u = -1; $display("u %0d %h", u, u);
    i = u; $display("i from u %0d", i);
    $display("i/u cmp %b %b", i < u, i < 0);
    i = -7; u = 2; $display("i/u div %0d", i / u);
    $display("i/i div %0d %0d", i / 2, i % 2);
    i = 'hffff_ffff; $display("hex lit %0d", i);
    i = 32'shffff_ffff; $display("s hex lit %0d", i);
    i = 8'hff; $display("8'hff %0d", i);
    i = 8'shff; $display("8'shff %0d", i);
    i = -8'shff; $display("-8'shff %0d", i);
    b = -1; i = b; $display("b -> i %0d", i);
    i = b + 1; $display("b + 1 %0d", i);
    i = b + 1'b1; $display("b + 1'b1 %0d", i);
    b = b + 1; $display("b wraps %0d", b);
    t = 64'hffff_ffff_ffff_ffff; $display("time max %0d", t);
    t = -1; $display("time neg %0d", t);
    t = 5; $display("time - %0d", t - 10);
    i = 5; $display("time vs int %b", t > i);
    i = 7; j = 3;
    $display("ops %0d %0d %0d %0d %0d", i + j, i - j, i * j, i / j, i % j);
    $display("bitops %0d %0d %0d %0d", i & j, i | j, i ^ j, ~i);
    $display("shifts %0d %0d %0d", i << 2, -i >> 28, -i >>> 1);
    $display("pow %0d %0d", i ** j, (-i) ** j);
    $display("cmp %b %b %b", i > j, -i > j, -i < 32'd3);
    $finish;
  end
endmodule
