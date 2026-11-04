// 64 words of 8, 1 read(s), async: inferred.
module mem64x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [20:0] sr;
  always @(posedge clk) sr <= {sr[19:0], si};
  reg [7:0] mem [0:63];
  always @(posedge clk) if (sr[20]) mem[sr[5:0]] <= sr[13:6];
  reg [7:0] q0;
  always @(posedge clk) q0 <= mem[sr[19:14]];
  always @(posedge clk) so <= ^q0;
endmodule
