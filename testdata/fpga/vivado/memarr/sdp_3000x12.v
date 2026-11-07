// 3000 words of 12: one write, one read on the clock.
module sdp_3000x12 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [37:0] sr;
  always @(posedge clk) sr <= {sr[36:0], si};
  reg [11:0] mem [0:2999];
  reg [11:0] q0;
  always @(posedge clk) if (sr[24]) mem[sr[11:0]] <= sr[23:12];
  always @(posedge clk) q0 <= mem[sr[36:25]];
  always @(posedge clk) so <= ^(q0) ^ sr[37];
endmodule
