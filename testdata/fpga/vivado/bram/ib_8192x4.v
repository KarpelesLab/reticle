// 8192 words of 4 to infer: one write, one registered read.
module ib_8192x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [31:0] sr;
  always @(posedge clk) sr <= {sr[30:0], si};
  reg [3:0] mem [0:8191];
  always @(posedge clk) if (sr[30]) mem[sr[12:0]] <= sr[16:13];
  reg [3:0] q0;
  always @(posedge clk) q0 <= mem[sr[29:17]];
  always @(posedge clk) so <= ^(q0) ^ sr[31];
endmodule
