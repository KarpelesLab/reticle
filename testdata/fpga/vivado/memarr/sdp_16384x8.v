// 16384 words of 8: one write, one read on the clock.
module sdp_16384x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [37:0] sr;
  always @(posedge clk) sr <= {sr[36:0], si};
  reg [7:0] mem [0:16383];
  reg [7:0] q0;
  always @(posedge clk) if (sr[22]) mem[sr[13:0]] <= sr[21:14];
  always @(posedge clk) q0 <= mem[sr[36:23]];
  always @(posedge clk) so <= ^(q0) ^ sr[37];
endmodule
