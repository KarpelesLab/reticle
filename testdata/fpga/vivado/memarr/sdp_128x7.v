// 128 words of 7: one write, one read on the clock.
module sdp_128x7 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [22:0] sr;
  always @(posedge clk) sr <= {sr[21:0], si};
  reg [6:0] mem [0:127];
  reg [6:0] q0;
  always @(posedge clk) if (sr[14]) mem[sr[6:0]] <= sr[13:7];
  always @(posedge clk) q0 <= mem[sr[21:15]];
  always @(posedge clk) so <= ^(q0) ^ sr[22];
endmodule
