// 32 words of 1 (sp_as).
module sp_as_32x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [7:0] sr;
  always @(posedge clk) sr <= {sr[6:0], si};
  reg [0:0] mem [0:31];
  reg [0:0] q0;
  always @(posedge clk) if (sr[6]) mem[sr[4:0]] <= sr[5];
  always @(*) q0 = mem[sr[4:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[7];
endmodule
