// 32 words of 7, 1 read(s), async: inferred.
module mem32x7 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [17:0] sr;
  always @(posedge clk) sr <= {sr[16:0], si};
  reg [6:0] mem [0:31];
  always @(posedge clk) if (sr[17]) mem[sr[4:0]] <= sr[11:5];
  reg [6:0] q0;
  always @(posedge clk) q0 <= mem[sr[16:12]];
  always @(posedge clk) so <= ^q0;
endmodule
