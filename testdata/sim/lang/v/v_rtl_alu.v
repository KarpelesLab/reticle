`timescale 1ns/1ps
module alu (input clk, input [3:0] op, input signed [7:0] a, input signed [7:0] b,
            output reg signed [7:0] y, output reg c, output reg z, output reg n, output reg v);
  reg signed [8:0] t;
  always @(posedge clk) begin
    c <= 1'b0; v <= 1'b0;
    case (op)
      4'd0: begin t = {a[7], a} + {b[7], b}; y <= t[7:0]; c <= ({1'b0, a} + {1'b0, b}) >> 8;
                  v <= (a[7] == b[7]) && (t[7] != a[7]); end
      4'd1: begin t = {a[7], a} - {b[7], b}; y <= t[7:0]; c <= a < b;
                  v <= (a[7] != b[7]) && (t[7] != a[7]); end
      4'd2: y <= a & b;
      4'd3: y <= a | b;
      4'd4: y <= a ^ b;
      4'd5: y <= ~a;
      4'd6: y <= a <<< b[2:0];
      4'd7: y <= a >>> b[2:0];
      4'd8: y <= $unsigned(a) >> b[2:0];
      4'd9: y <= (a < b) ? 8'sd1 : 8'sd0;
      4'd10: y <= ($unsigned(a) < $unsigned(b)) ? 8'sd1 : 8'sd0;
      4'd11: y <= a * b;
      default: y <= 8'sd0;
    endcase
  end
  always @* begin z = (y == 0); n = y[7]; end
endmodule
module tb;
  reg clk;
  reg [3:0] op;
  reg signed [7:0] a, b;
  wire signed [7:0] y;
  wire c, z, n, v;
  integer i, j;
  reg signed [7:0] av [0:4];
  alu dut (.clk(clk), .op(op), .a(a), .b(b), .y(y), .c(c), .z(z), .n(n), .v(v));
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin
    av[0] = 8'sd0; av[1] = 8'sd100; av[2] = -8'sd100; av[3] = 8'sd127; av[4] = -8'sd128;
    for (op = 0; op < 12; op = op + 1)
      for (i = 0; i < 5; i = i + 1) begin
        j = (i + 2) % 5;
        a = av[i]; b = av[j];
        if (op >= 6 && op <= 8) b = i + 1;
        @(posedge clk); #1;
        $display("op=%0d a=%0d b=%0d y=%0d (%h) c=%b z=%b n=%b v=%b", op, a, b, y, y, c, z, n, v);
      end
    $finish;
  end
endmodule
