// 64 words of 4 (sp_as).
module sp_as_64x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [11:0] sr;
  always @(posedge clk) sr <= {sr[10:0], si};
  reg [3:0] mem [0:63];
  reg [3:0] q0;
  always @(posedge clk) if (sr[10]) mem[sr[5:0]] <= sr[9:6];
  always @(*) q0 = mem[sr[5:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[11];
endmodule
