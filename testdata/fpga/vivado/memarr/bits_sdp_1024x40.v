// 1024 words of 40 (sdp), read under a mask.
module bits_sdp_1024x40 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [61:0] sr;
  always @(posedge clk) sr <= {sr[60:0], si};
  reg [39:0] mem [0:1023];
  reg [39:0] q0;
  always @(posedge clk) if (sr[50]) mem[sr[9:0]] <= sr[49:10];
  always @(posedge clk) q0 <= mem[sr[60:51]];
  always @(posedge clk) so <= ^((q0) & sr[39:0]) ^ sr[61];
endmodule
