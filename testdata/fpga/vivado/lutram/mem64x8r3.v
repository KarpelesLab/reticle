// 64 words of 8, 3 read(s), async: inferred.
module mem64x8r3 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [32:0] sr;
  always @(posedge clk) sr <= {sr[31:0], si};
  reg [7:0] mem [0:63];
  always @(posedge clk) if (sr[32]) mem[sr[5:0]] <= sr[13:6];
  reg [7:0] q0;
  always @(posedge clk) q0 <= mem[sr[19:14]];
  reg [7:0] q1;
  always @(posedge clk) q1 <= mem[sr[25:20]];
  reg [7:0] q2;
  always @(posedge clk) q2 <= mem[sr[31:26]];
  always @(posedge clk) so <= ^q0 ^ ^q1 ^ ^q2;
endmodule
