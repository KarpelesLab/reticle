// 32 words of 32, 1 read(s), async: inferred.
module mem32x32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [42:0] sr;
  always @(posedge clk) sr <= {sr[41:0], si};
  reg [31:0] mem [0:31];
  always @(posedge clk) if (sr[42]) mem[sr[4:0]] <= sr[36:5];
  reg [31:0] q0;
  always @(posedge clk) q0 <= mem[sr[41:37]];
  always @(posedge clk) so <= ^q0;
endmodule
