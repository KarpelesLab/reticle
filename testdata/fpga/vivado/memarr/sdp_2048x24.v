// 2048 words of 24: one write, one read on the clock.
module sdp_2048x24 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [47:0] sr;
  always @(posedge clk) sr <= {sr[46:0], si};
  reg [23:0] mem [0:2047];
  reg [23:0] q0;
  always @(posedge clk) if (sr[35]) mem[sr[10:0]] <= sr[34:11];
  always @(posedge clk) q0 <= mem[sr[46:36]];
  always @(posedge clk) so <= ^(q0) ^ sr[47];
endmodule
