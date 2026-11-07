// 2048 words of 36: one write, one read on the clock.
module sdp_2048x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [59:0] sr;
  always @(posedge clk) sr <= {sr[58:0], si};
  reg [35:0] mem [0:2047];
  reg [35:0] q0;
  always @(posedge clk) if (sr[47]) mem[sr[10:0]] <= sr[46:11];
  always @(posedge clk) q0 <= mem[sr[58:48]];
  always @(posedge clk) so <= ^(q0) ^ sr[59];
endmodule
