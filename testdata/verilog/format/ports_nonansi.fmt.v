// Verilog-1995 style port lists: names only, with the directions and
// types declared in the body, plus port expressions and explicit names.
module adder (a, b, cin, sum, cout);
  input [3:0] a, b;
  input cin;
  output [3:0] sum;
  output cout;
  assign {cout, sum} = a + b + cin;
endmodule

module regs (clk, d, q);
  input clk;
  input [7:0] d;
  output [7:0] q;
  reg [7:0] q;
  always @(posedge clk) q <= d;
endmodule

module regs2 (clk, d, q);
  input clk;
  input [7:0] d;
  output reg [7:0] q;
  always @(posedge clk) q <= d;
endmodule

module split (.hi(bus[7:4]), .lo(bus[3:0]), .unused(), {x, y}, z[1:0], , w);
  inout [7:0] bus;
  input x, y;
  input [1:0] z;
  output w;
  wire w = 1'b0;
endmodule

module none;
endmodule

module empty_parens ();
endmodule
