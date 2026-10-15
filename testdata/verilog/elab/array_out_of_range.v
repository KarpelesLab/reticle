// Fixed elements past the end of an array: a write is dropped and a read
// is x, as a configuration with fewer entries than the code names leaves
// them.
module array_out_of_range(input [7:0] a, input [7:0] b, input [7:0] c, output [7:0] y0, output [7:0] y2);
  wire [7:0] m [0:1];
  assign m[0] = a;
  assign m[1] = b;
  assign m[2] = c;
  assign y0 = m[0];
  assign y2 = m[2];
endmodule
