`timescale 1ns/1ps
module tb;
  reg signed [7:0] a, b;
  reg [7:0] ua, ub;
  reg signed [15:0] p;
  reg [15:0] up;
  reg signed [3:0] n;
  initial begin
    a = -8'sd7; b = 8'sd9; ua = 8'd249; ub = 8'd9;
    p = a * b; $display("s*s %0d %h", p, p);
    up = ua * ub; $display("u*u %0d %h", up, up);
    p = a * ub; $display("s*u %0d %h", p, p);
    p = $signed(ua) * b; $display("$signed*s %0d", p);
    p = a * $signed(ub); $display("s*$signed %0d", p);
    p = $signed({1'b0, ua}) * b; $display("pos signed %0d", p);
    p = a / 8'sd2; $display("s/s %0d", p);
    p = a / 8'd2; $display("s/u %0d", p);
    p = a % 8'sd4; $display("s%%s %0d", p);
    p = a % 8'd4; $display("s%%u %0d", p);
    p = -a; $display("neg %0d", p);
    p = a[7:0] * b; $display("part unsigned %0d", p);
    p = a[3:0]; $display("part assign %0d", p);
    n = a[3:0]; $display("part to signed %0d", n);
    p = n * n; $display("n*n %0d", p);
    p = n * 4'd2; $display("n*u %0d", p);
    p = (a + b) >>> 1; $display("sum>>>1 %0d", p);
    p = (a - b) >>> 1; $display("diff>>>1 %0d", p);
    p = {a} * b; $display("concat mult %0d", p);
    p = a * b * 2; $display("chain %0d", p);
    p = a ** 2; $display("pow %0d", p);
    p = a ** 3; $display("pow3 %0d", p);
    p = 8'sd2 ** -8'sd1; $display("pow neg exp %0d", p);
    p = -8'sd1 ** -8'sd1; $display("pow -1^-1 %0d", p);
    p = 8'sd0 ** -8'sd1; $display("pow 0^-1 %b", p);
    up = a; $display("sext to unsigned %h", up);
    up = a + 0; $display("a+0 %h", up);
    up = a + 8'd0; $display("a+8'd0 %h", up);
    $finish;
  end
endmodule
