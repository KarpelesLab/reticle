// 512 words of 72 (sdp), read under a mask.
module bits_sdp_512x72 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [91:0] sr;
  always @(posedge clk) sr <= {sr[90:0], si};
  reg [71:0] mem [0:511];
  reg [71:0] q0;
  always @(posedge clk) if (sr[81]) mem[sr[8:0]] <= sr[80:9];
  always @(posedge clk) q0 <= mem[sr[90:82]];
  always @(posedge clk) so <= ^((q0) & sr[71:0]) ^ sr[91];
endmodule
