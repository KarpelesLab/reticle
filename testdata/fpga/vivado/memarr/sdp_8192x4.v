// 8192 words of 4: one write, one read on the clock.
module sdp_8192x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [31:0] sr;
  always @(posedge clk) sr <= {sr[30:0], si};
  reg [3:0] mem [0:8191];
  reg [3:0] q0;
  always @(posedge clk) if (sr[17]) mem[sr[12:0]] <= sr[16:13];
  always @(posedge clk) q0 <= mem[sr[30:18]];
  always @(posedge clk) so <= ^(q0) ^ sr[31];
endmodule
