// 256 words of 2, 1 read(s), async: inferred.
module mem256x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [18:0] sr;
  always @(posedge clk) sr <= {sr[17:0], si};
  reg [1:0] mem [0:255];
  always @(posedge clk) if (sr[18]) mem[sr[7:0]] <= sr[9:8];
  reg [1:0] q0;
  always @(posedge clk) q0 <= mem[sr[17:10]];
  always @(posedge clk) so <= ^q0;
endmodule
