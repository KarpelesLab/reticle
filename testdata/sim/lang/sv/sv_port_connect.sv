`timescale 1ns/1ps
module mix (input logic [7:0] a, input logic [7:0] b, input logic sel, output logic [7:0] y, output logic [8:0] sum);
  assign y = sel ? a : b;
  assign sum = a + b;
endmodule

module tb;
  logic [7:0] a, b;
  logic sel;
  logic [7:0] y;
  logic [8:0] sum;
  logic [7:0] y2;
  logic [8:0] sum2;
  logic [7:0] y3;
  mix u_star (.*);
  mix u_name (.a, .b, .sel, .y(y2), .sum(sum2));
  mix u_mix (.sel(1'b0), .*, .y(y3), .sum());
  initial begin
    a = 8'd10; b = 8'd250; sel = 1;
    #1 $display("PC1 y=%0d sum=%0d y2=%0d sum2=%0d y3=%0d", y, sum, y2, sum2, y3);
    sel = 0;
    #1 $display("PC2 y=%0d y2=%0d y3=%0d", y, y2, y3);
    $finish;
  end
endmodule
