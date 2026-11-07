`timescale 1ns/1ps
module tb;
  reg [3:0] a, b;
  reg c;
  integer i;
  initial begin
    a = 4'b10x1; b = 4'd3;
    $display("add %b sub %b mul %b", a + b, a - b, a * b);
    $display("div %b mod %b", a / b, b % a);
    $display("neg %b", -a);
    $display("cmp %b %b %b", a < b, a == b, a === 4'b10x1);
    $display("eqz %b %b", 4'bz == 4'bz, 4'bz === 4'bz);
    $display("neq %b %b", 4'b1x00 != 4'b0x00, 4'b1x00 !== 4'b1x00);
    c = 1'bx;
    if (c) $display("if x taken"); else $display("if x else");
    if (!c) $display("if !x taken"); else $display("if !x else");
    c = 1'bz;
    if (c) $display("if z taken"); else $display("if z else");
    if (4'b00x0) $display("if vec x taken"); else $display("if vec x else");
    if (4'b01x0) $display("if vec 1x taken"); else $display("if vec 1x else");
    i = 'bx; $display("int x %d %h", i, i);
    i = 'bx; i = i + 1; $display("int x+1 %d", i);
    c = 1'bx;
    while (c) begin $display("while x"); c = 0; end
    $display("while done");
    i = 0;
    repeat (2'bx1) i = i + 1;
    $display("repeat x %0d", i);
    a = 4'bxxxx;
    case (a)
      4'bxxxx: $display("case x matched");
      default: $display("case x default");
    endcase
    a = 4'b1z00;
    case (a)
      4'b1000: $display("case z 1000");
      4'b1z00: $display("case z exact");
      default: $display("case z default");
    endcase
    $display("concat x %b", {2'bx1, 2'bz0});
    $display("shiftin %b", 4'bx000 >> 1);
    $display("red %b %b", ^4'b1x00, &4'b0x00);
    $display("pow x %b", a ** 2);
    $display("cond merge %b", 1'bx ? 4'b1010 : 4'b1001);
    $finish;
  end
endmodule
