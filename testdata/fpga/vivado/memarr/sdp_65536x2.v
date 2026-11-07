// 65536 words of 2 (sdp).
module sdp_65536x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [35:0] sr;
  always @(posedge clk) sr <= {sr[34:0], si};
  reg [1:0] mem [0:65535];
  reg [1:0] q0;
  always @(posedge clk) if (sr[18]) mem[sr[15:0]] <= sr[17:16];
  always @(posedge clk) q0 <= mem[sr[34:19]];
  always @(posedge clk) so <= ^(q0) ^ sr[35];
endmodule
