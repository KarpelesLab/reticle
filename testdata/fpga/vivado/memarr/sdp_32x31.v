// 32 words of 31: one write, one read on the clock.
module sdp_32x31 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [42:0] sr;
  always @(posedge clk) sr <= {sr[41:0], si};
  reg [30:0] mem [0:31];
  reg [30:0] q0;
  always @(posedge clk) if (sr[36]) mem[sr[4:0]] <= sr[35:5];
  always @(posedge clk) q0 <= mem[sr[41:37]];
  always @(posedge clk) so <= ^(q0) ^ sr[42];
endmodule
