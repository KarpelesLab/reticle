// 64 words of 8, 2 read(s), async: inferred.
module mem64x8r2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [26:0] sr;
  always @(posedge clk) sr <= {sr[25:0], si};
  reg [7:0] mem [0:63];
  always @(posedge clk) if (sr[26]) mem[sr[5:0]] <= sr[13:6];
  reg [7:0] q0;
  always @(posedge clk) q0 <= mem[sr[19:14]];
  reg [7:0] q1;
  always @(posedge clk) q1 <= mem[sr[25:20]];
  always @(posedge clk) so <= ^q0 ^ ^q1;
endmodule
