// 16 words of 8, 1 read(s), async: inferred.
module mem16x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [16:0] sr;
  always @(posedge clk) sr <= {sr[15:0], si};
  reg [7:0] mem [0:15];
  always @(posedge clk) if (sr[16]) mem[sr[3:0]] <= sr[11:4];
  reg [7:0] q0;
  always @(posedge clk) q0 <= mem[sr[15:12]];
  always @(posedge clk) so <= ^q0;
endmodule
