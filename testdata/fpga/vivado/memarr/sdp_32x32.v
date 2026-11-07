// 32 words of 32: one write, one read on the clock.
module sdp_32x32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [43:0] sr;
  always @(posedge clk) sr <= {sr[42:0], si};
  reg [31:0] mem [0:31];
  reg [31:0] q0;
  always @(posedge clk) if (sr[37]) mem[sr[4:0]] <= sr[36:5];
  always @(posedge clk) q0 <= mem[sr[42:38]];
  always @(posedge clk) so <= ^(q0) ^ sr[43];
endmodule
