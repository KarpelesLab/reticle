// 65536 words of 4 (sdp).
module sdp_65536x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [37:0] sr;
  always @(posedge clk) sr <= {sr[36:0], si};
  reg [3:0] mem [0:65535];
  reg [3:0] q0;
  always @(posedge clk) if (sr[20]) mem[sr[15:0]] <= sr[19:16];
  always @(posedge clk) q0 <= mem[sr[36:21]];
  always @(posedge clk) so <= ^(q0) ^ sr[37];
endmodule
