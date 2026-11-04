// 128 words of 1, 1 read(s), async: inferred.
module mem128x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [15:0] sr;
  always @(posedge clk) sr <= {sr[14:0], si};
  reg [0:0] mem [0:127];
  always @(posedge clk) if (sr[15]) mem[sr[6:0]] <= sr[7:7];
  reg [0:0] q0;
  always @(posedge clk) q0 <= mem[sr[14:8]];
  always @(posedge clk) so <= ^q0;
endmodule
