// 64 words of 3, 1 read(s), async: inferred.
module mem64x3 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [15:0] sr;
  always @(posedge clk) sr <= {sr[14:0], si};
  reg [2:0] mem [0:63];
  always @(posedge clk) if (sr[15]) mem[sr[5:0]] <= sr[8:6];
  reg [2:0] q0;
  always @(posedge clk) q0 <= mem[sr[14:9]];
  always @(posedge clk) so <= ^q0;
endmodule
