// 16 words of 1, 1 read(s), async: inferred.
module mem16x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  reg [0:0] mem [0:15];
  always @(posedge clk) if (sr[9]) mem[sr[3:0]] <= sr[4:4];
  reg [0:0] q0;
  always @(posedge clk) q0 <= mem[sr[8:5]];
  always @(posedge clk) so <= ^q0;
endmodule
