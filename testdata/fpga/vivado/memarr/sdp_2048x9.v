// 2048 words of 9: one write, one read on the clock.
module sdp_2048x9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [32:0] sr;
  always @(posedge clk) sr <= {sr[31:0], si};
  reg [8:0] mem [0:2047];
  reg [8:0] q0;
  always @(posedge clk) if (sr[20]) mem[sr[10:0]] <= sr[19:11];
  always @(posedge clk) q0 <= mem[sr[31:21]];
  always @(posedge clk) so <= ^(q0) ^ sr[32];
endmodule
