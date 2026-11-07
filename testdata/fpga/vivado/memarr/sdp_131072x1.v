// 131072 words of 1 (sdp).
module sdp_131072x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [36:0] sr;
  always @(posedge clk) sr <= {sr[35:0], si};
  reg [0:0] mem [0:131071];
  reg [0:0] q0;
  always @(posedge clk) if (sr[18]) mem[sr[16:0]] <= sr[17];
  always @(posedge clk) q0 <= mem[sr[35:19]];
  always @(posedge clk) so <= ^(q0) ^ sr[36];
endmodule
