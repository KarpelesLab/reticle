// 2048 words of 18 to infer: one write, one registered read.
module ib_2048x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [41:0] sr;
  always @(posedge clk) sr <= {sr[40:0], si};
  reg [17:0] mem [0:2047];
  always @(posedge clk) if (sr[40]) mem[sr[10:0]] <= sr[28:11];
  reg [17:0] q0;
  always @(posedge clk) q0 <= mem[sr[39:29]];
  always @(posedge clk) so <= ^(q0) ^ sr[41];
endmodule
