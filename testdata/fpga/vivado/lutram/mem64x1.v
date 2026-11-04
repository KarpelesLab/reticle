// 64 words of 1, 1 read(s), async: inferred.
module mem64x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [13:0] sr;
  always @(posedge clk) sr <= {sr[12:0], si};
  reg [0:0] mem [0:63];
  always @(posedge clk) if (sr[13]) mem[sr[5:0]] <= sr[6:6];
  reg [0:0] q0;
  always @(posedge clk) q0 <= mem[sr[12:7]];
  always @(posedge clk) so <= ^q0;
endmodule
