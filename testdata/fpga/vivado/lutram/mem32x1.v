// 32 words of 1, 1 read(s), async: inferred.
module mem32x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [11:0] sr;
  always @(posedge clk) sr <= {sr[10:0], si};
  reg [0:0] mem [0:31];
  always @(posedge clk) if (sr[11]) mem[sr[4:0]] <= sr[5:5];
  reg [0:0] q0;
  always @(posedge clk) q0 <= mem[sr[10:6]];
  always @(posedge clk) so <= ^q0;
endmodule
