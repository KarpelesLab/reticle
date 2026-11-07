// 40000 words of 1 (sdp).
module sdp_40000x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [34:0] sr;
  always @(posedge clk) sr <= {sr[33:0], si};
  reg [0:0] mem [0:39999];
  reg [0:0] q0;
  always @(posedge clk) if (sr[17]) mem[sr[15:0]] <= sr[16];
  always @(posedge clk) q0 <= mem[sr[33:18]];
  always @(posedge clk) so <= ^(q0) ^ sr[34];
endmodule
