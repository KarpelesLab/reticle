// 32 words of 8, 4 read(s), async: inferred.
module mem32x8r4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [33:0] sr;
  always @(posedge clk) sr <= {sr[32:0], si};
  reg [7:0] mem [0:31];
  always @(posedge clk) if (sr[33]) mem[sr[4:0]] <= sr[12:5];
  reg [7:0] q0;
  always @(posedge clk) q0 <= mem[sr[17:13]];
  reg [7:0] q1;
  always @(posedge clk) q1 <= mem[sr[22:18]];
  reg [7:0] q2;
  always @(posedge clk) q2 <= mem[sr[27:23]];
  reg [7:0] q3;
  always @(posedge clk) q3 <= mem[sr[32:28]];
  always @(posedge clk) so <= ^q0 ^ ^q1 ^ ^q2 ^ ^q3;
endmodule
