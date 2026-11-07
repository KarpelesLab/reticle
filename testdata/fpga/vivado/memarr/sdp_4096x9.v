// 4096 words of 9: one write, one read on the clock.
module sdp_4096x9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [34:0] sr;
  always @(posedge clk) sr <= {sr[33:0], si};
  reg [8:0] mem [0:4095];
  reg [8:0] q0;
  always @(posedge clk) if (sr[21]) mem[sr[11:0]] <= sr[20:12];
  always @(posedge clk) q0 <= mem[sr[33:22]];
  always @(posedge clk) so <= ^(q0) ^ sr[34];
endmodule
