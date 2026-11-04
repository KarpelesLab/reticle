// 4096 words of 9 to infer: one write, one registered read.
module ib_4096x9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [34:0] sr;
  always @(posedge clk) sr <= {sr[33:0], si};
  reg [8:0] mem [0:4095];
  always @(posedge clk) if (sr[33]) mem[sr[11:0]] <= sr[20:12];
  reg [8:0] q0;
  always @(posedge clk) q0 <= mem[sr[32:21]];
  always @(posedge clk) so <= ^(q0) ^ sr[34];
endmodule
