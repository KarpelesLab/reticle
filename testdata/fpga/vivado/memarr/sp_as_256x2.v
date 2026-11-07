// 256 words of 2 (sp_as).
module sp_as_256x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [11:0] sr;
  always @(posedge clk) sr <= {sr[10:0], si};
  reg [1:0] mem [0:255];
  reg [1:0] q0;
  always @(posedge clk) if (sr[10]) mem[sr[7:0]] <= sr[9:8];
  always @(*) q0 = mem[sr[7:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[11];
endmodule
