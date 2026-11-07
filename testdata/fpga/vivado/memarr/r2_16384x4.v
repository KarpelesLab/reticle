// 16384 words of 4: one write and two reads.
module r2_16384x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [47:0] sr;
  always @(posedge clk) sr <= {sr[46:0], si};
  reg [3:0] mem [0:16383];
  reg [3:0] q0;
  always @(posedge clk) if (sr[18]) mem[sr[13:0]] <= sr[17:14];
  always @(posedge clk) q0 <= mem[sr[32:19]];
  reg [3:0] p0;
  always @(posedge clk) p0 <= mem[sr[46:33]];
  always @(posedge clk) so <= ^(q0 ^ p0) ^ sr[47];
endmodule
