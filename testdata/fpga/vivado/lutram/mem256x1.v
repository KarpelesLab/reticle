// 256 words of 1, 1 read(s), async: inferred.
module mem256x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [17:0] sr;
  always @(posedge clk) sr <= {sr[16:0], si};
  reg [0:0] mem [0:255];
  always @(posedge clk) if (sr[17]) mem[sr[7:0]] <= sr[8:8];
  reg [0:0] q0;
  always @(posedge clk) q0 <= mem[sr[16:9]];
  always @(posedge clk) so <= ^q0;
endmodule
