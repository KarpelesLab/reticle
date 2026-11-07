// 32 words of 4 (sp_as).
module sp_as_32x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [10:0] sr;
  always @(posedge clk) sr <= {sr[9:0], si};
  reg [3:0] mem [0:31];
  reg [3:0] q0;
  always @(posedge clk) if (sr[9]) mem[sr[4:0]] <= sr[8:5];
  always @(*) q0 = mem[sr[4:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[10];
endmodule
