// 32 words of 6, 1 read(s), async: inferred.
module mem32x6 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [16:0] sr;
  always @(posedge clk) sr <= {sr[15:0], si};
  reg [5:0] mem [0:31];
  always @(posedge clk) if (sr[16]) mem[sr[4:0]] <= sr[10:5];
  reg [5:0] q0;
  always @(posedge clk) q0 <= mem[sr[15:11]];
  always @(posedge clk) so <= ^q0;
endmodule
