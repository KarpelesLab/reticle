`timescale 1ns/1ps
module add4 (input [3:0] a, output [4:0] s);
  assign s = a + 1;
endmodule
module tb;
  reg [3:0] a;
  wire [2:0] n;
  wire [7:0] w;
  add4 u3 (.a(a), .s(n));
  add4 u4 (.a(a), .s(w));
  add4 ua [1:0] (.a({a, a}), .s());
  initial begin
    a = 4'd15;
    #1 $display("narrow=%b wide=%b %h", n, w, w);
    $display("ua[1].s=%0d", ua[1].s);
    $finish;
  end
endmodule
