`timescale 1ns/1ps
module leaf (input [3:0] a, output [3:0] y);
  assign y = a + 1;
endmodule
module tb;
  reg [3:0] a;
  wire [3:0] y [0:1];
  genvar g;
  generate
    for (g = 0; g < 2; g = g + 1) begin : blk
      leaf u (.a(a), .y(y[g]));
      wire [3:0] w2 = a;
    end
  endgenerate
  initial begin
    a = 4'd5;
    #1 $display("y %0d %0d", y[0], y[1]);
    $finish;
  end
endmodule
