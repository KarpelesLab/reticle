// 64 words of 16: one write, one read on the clock.
module sdp_64x16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [29:0] sr;
  always @(posedge clk) sr <= {sr[28:0], si};
  reg [15:0] mem [0:63];
  reg [15:0] q0;
  always @(posedge clk) if (sr[22]) mem[sr[5:0]] <= sr[21:6];
  always @(posedge clk) q0 <= mem[sr[28:23]];
  always @(posedge clk) so <= ^(q0) ^ sr[29];
endmodule
