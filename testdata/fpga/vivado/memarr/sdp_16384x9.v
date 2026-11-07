// 16384 words of 9: one write, one read on the clock.
module sdp_16384x9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [38:0] sr;
  always @(posedge clk) sr <= {sr[37:0], si};
  reg [8:0] mem [0:16383];
  reg [8:0] q0;
  always @(posedge clk) if (sr[23]) mem[sr[13:0]] <= sr[22:14];
  always @(posedge clk) q0 <= mem[sr[37:24]];
  always @(posedge clk) so <= ^(q0) ^ sr[38];
endmodule
