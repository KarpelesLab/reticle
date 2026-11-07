// 4096 words of 32: one write, one read on the clock.
module sdp_4096x32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [57:0] sr;
  always @(posedge clk) sr <= {sr[56:0], si};
  reg [31:0] mem [0:4095];
  reg [31:0] q0;
  always @(posedge clk) if (sr[44]) mem[sr[11:0]] <= sr[43:12];
  always @(posedge clk) q0 <= mem[sr[56:45]];
  always @(posedge clk) so <= ^(q0) ^ sr[57];
endmodule
