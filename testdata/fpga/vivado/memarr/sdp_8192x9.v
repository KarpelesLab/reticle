// 8192 words of 9: one write, one read on the clock.
module sdp_8192x9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [36:0] sr;
  always @(posedge clk) sr <= {sr[35:0], si};
  reg [8:0] mem [0:8191];
  reg [8:0] q0;
  always @(posedge clk) if (sr[22]) mem[sr[12:0]] <= sr[21:13];
  always @(posedge clk) q0 <= mem[sr[35:23]];
  always @(posedge clk) so <= ^(q0) ^ sr[36];
endmodule
