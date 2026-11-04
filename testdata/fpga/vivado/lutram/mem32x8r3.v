// 32 words of 8, 3 read(s), async: inferred.
module mem32x8r3 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [28:0] sr;
  always @(posedge clk) sr <= {sr[27:0], si};
  reg [7:0] mem [0:31];
  always @(posedge clk) if (sr[28]) mem[sr[4:0]] <= sr[12:5];
  reg [7:0] q0;
  always @(posedge clk) q0 <= mem[sr[17:13]];
  reg [7:0] q1;
  always @(posedge clk) q1 <= mem[sr[22:18]];
  reg [7:0] q2;
  always @(posedge clk) q2 <= mem[sr[27:23]];
  always @(posedge clk) so <= ^q0 ^ ^q1 ^ ^q2;
endmodule
