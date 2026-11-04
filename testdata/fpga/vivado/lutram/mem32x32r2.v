// 32 words of 32, 2 read(s), async: inferred.
module mem32x32r2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [47:0] sr;
  always @(posedge clk) sr <= {sr[46:0], si};
  reg [31:0] mem [0:31];
  always @(posedge clk) if (sr[47]) mem[sr[4:0]] <= sr[36:5];
  reg [31:0] q0;
  always @(posedge clk) q0 <= mem[sr[41:37]];
  reg [31:0] q1;
  always @(posedge clk) q1 <= mem[sr[46:42]];
  always @(posedge clk) so <= ^q0 ^ ^q1;
endmodule
