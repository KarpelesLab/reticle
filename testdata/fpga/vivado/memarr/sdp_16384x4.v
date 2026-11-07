// 16384 words of 4: one write, one read on the clock.
module sdp_16384x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [33:0] sr;
  always @(posedge clk) sr <= {sr[32:0], si};
  reg [3:0] mem [0:16383];
  reg [3:0] q0;
  always @(posedge clk) if (sr[18]) mem[sr[13:0]] <= sr[17:14];
  always @(posedge clk) q0 <= mem[sr[32:19]];
  always @(posedge clk) so <= ^(q0) ^ sr[33];
endmodule
