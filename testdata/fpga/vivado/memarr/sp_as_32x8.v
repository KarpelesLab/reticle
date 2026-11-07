// 32 words of 8 (sp_as).
module sp_as_32x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [14:0] sr;
  always @(posedge clk) sr <= {sr[13:0], si};
  reg [7:0] mem [0:31];
  reg [7:0] q0;
  always @(posedge clk) if (sr[13]) mem[sr[4:0]] <= sr[12:5];
  always @(*) q0 = mem[sr[4:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[14];
endmodule
