// 8192 words of 16: one write, one read on the clock.
module sdp_8192x16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [43:0] sr;
  always @(posedge clk) sr <= {sr[42:0], si};
  reg [15:0] mem [0:8191];
  reg [15:0] q0;
  always @(posedge clk) if (sr[29]) mem[sr[12:0]] <= sr[28:13];
  always @(posedge clk) q0 <= mem[sr[42:30]];
  always @(posedge clk) so <= ^(q0) ^ sr[43];
endmodule
