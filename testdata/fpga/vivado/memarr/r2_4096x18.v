// 4096 words of 18: one write and two reads.
module r2_4096x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [55:0] sr;
  always @(posedge clk) sr <= {sr[54:0], si};
  reg [17:0] mem [0:4095];
  reg [17:0] q0;
  always @(posedge clk) if (sr[30]) mem[sr[11:0]] <= sr[29:12];
  always @(posedge clk) q0 <= mem[sr[42:31]];
  reg [17:0] p0;
  always @(posedge clk) p0 <= mem[sr[54:43]];
  always @(posedge clk) so <= ^(q0 ^ p0) ^ sr[55];
endmodule
