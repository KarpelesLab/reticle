// 32768 words of 1 to infer: one write, one registered read.
module ib_32768x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [32:0] sr;
  always @(posedge clk) sr <= {sr[31:0], si};
  reg [0:0] mem [0:32767];
  always @(posedge clk) if (sr[31]) mem[sr[14:0]] <= sr[15:15];
  reg [0:0] q0;
  always @(posedge clk) q0 <= mem[sr[30:16]];
  always @(posedge clk) so <= ^(q0) ^ sr[32];
endmodule
