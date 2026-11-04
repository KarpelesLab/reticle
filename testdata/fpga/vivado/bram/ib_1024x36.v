// 1024 words of 36 to infer: one write, one registered read.
module ib_1024x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [57:0] sr;
  always @(posedge clk) sr <= {sr[56:0], si};
  reg [35:0] mem [0:1023];
  always @(posedge clk) if (sr[56]) mem[sr[9:0]] <= sr[45:10];
  reg [35:0] q0;
  always @(posedge clk) q0 <= mem[sr[55:46]];
  always @(posedge clk) so <= ^(q0) ^ sr[57];
endmodule
