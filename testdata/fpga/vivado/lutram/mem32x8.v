// 32 words of 8, 1 read(s), async: inferred.
module mem32x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [18:0] sr;
  always @(posedge clk) sr <= {sr[17:0], si};
  reg [7:0] mem [0:31];
  always @(posedge clk) if (sr[18]) mem[sr[4:0]] <= sr[12:5];
  reg [7:0] q0;
  always @(posedge clk) q0 <= mem[sr[17:13]];
  always @(posedge clk) so <= ^q0;
endmodule
