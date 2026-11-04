// 128 words of 4, 2 read(s), async: inferred.
module mem128x4r2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [25:0] sr;
  always @(posedge clk) sr <= {sr[24:0], si};
  reg [3:0] mem [0:127];
  always @(posedge clk) if (sr[25]) mem[sr[6:0]] <= sr[10:7];
  reg [3:0] q0;
  always @(posedge clk) q0 <= mem[sr[17:11]];
  reg [3:0] q1;
  always @(posedge clk) q1 <= mem[sr[24:18]];
  always @(posedge clk) so <= ^q0 ^ ^q1;
endmodule
