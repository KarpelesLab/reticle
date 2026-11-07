// 4096 words of 8 written, read 32 bits at a time.
module asym_4096x8_r32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [31:0] sr;
  always @(posedge clk) sr <= {sr[30:0], si};
  reg [7:0] mem [0:4095];
  always @(posedge clk) if (sr[20]) mem[sr[11:0]] <= sr[19:12];
  reg [31:0] q0;
  integer i;
  always @(posedge clk)
    for (i = 0; i < 4; i = i + 1)
      q0[i * 8 +: 8] <= mem[{sr[30:21], i[1:0]}];
  always @(posedge clk) so <= ^(q0) ^ sr[31];
endmodule
