// 2048 words of 9 written, read 36 bits at a time.
module asym_2048x9_r36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [30:0] sr;
  always @(posedge clk) sr <= {sr[29:0], si};
  reg [8:0] mem [0:2047];
  always @(posedge clk) if (sr[20]) mem[sr[10:0]] <= sr[19:11];
  reg [35:0] q0;
  integer i;
  always @(posedge clk)
    for (i = 0; i < 4; i = i + 1)
      q0[i * 9 +: 9] <= mem[{sr[29:21], i[1:0]}];
  always @(posedge clk) so <= ^(q0) ^ sr[30];
endmodule
