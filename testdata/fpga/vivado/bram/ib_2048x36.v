// 2048 words of 36 to infer: one write, one registered read.
module ib_2048x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [59:0] sr;
  always @(posedge clk) sr <= {sr[58:0], si};
  reg [35:0] mem [0:2047];
  always @(posedge clk) if (sr[58]) mem[sr[10:0]] <= sr[46:11];
  reg [35:0] q0;
  always @(posedge clk) q0 <= mem[sr[57:47]];
  always @(posedge clk) so <= ^(q0) ^ sr[59];
endmodule
