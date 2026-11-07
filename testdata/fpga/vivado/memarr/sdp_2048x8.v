// 2048 words of 8: one write, one read on the clock.
module sdp_2048x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [31:0] sr;
  always @(posedge clk) sr <= {sr[30:0], si};
  reg [7:0] mem [0:2047];
  reg [7:0] q0;
  always @(posedge clk) if (sr[19]) mem[sr[10:0]] <= sr[18:11];
  always @(posedge clk) q0 <= mem[sr[30:20]];
  always @(posedge clk) so <= ^(q0) ^ sr[31];
endmodule
