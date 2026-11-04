// 128 words of 2, 1 read(s), async: inferred.
module mem128x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [16:0] sr;
  always @(posedge clk) sr <= {sr[15:0], si};
  reg [1:0] mem [0:127];
  always @(posedge clk) if (sr[16]) mem[sr[6:0]] <= sr[8:7];
  reg [1:0] q0;
  always @(posedge clk) q0 <= mem[sr[15:9]];
  always @(posedge clk) so <= ^q0;
endmodule
