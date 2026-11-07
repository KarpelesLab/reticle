// 8192 words of 8: one write, one read on the clock.
module sdp_8192x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [35:0] sr;
  always @(posedge clk) sr <= {sr[34:0], si};
  reg [7:0] mem [0:8191];
  reg [7:0] q0;
  always @(posedge clk) if (sr[21]) mem[sr[12:0]] <= sr[20:13];
  always @(posedge clk) q0 <= mem[sr[34:22]];
  always @(posedge clk) so <= ^(q0) ^ sr[35];
endmodule
