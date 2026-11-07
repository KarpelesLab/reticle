// 64 words of 15: one write, one read on the clock.
module sdp_64x15 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [28:0] sr;
  always @(posedge clk) sr <= {sr[27:0], si};
  reg [14:0] mem [0:63];
  reg [14:0] q0;
  always @(posedge clk) if (sr[21]) mem[sr[5:0]] <= sr[20:6];
  always @(posedge clk) q0 <= mem[sr[27:22]];
  always @(posedge clk) so <= ^(q0) ^ sr[28];
endmodule
