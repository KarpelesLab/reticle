// 512 words of 8: one write, one read on the clock.
module sdp_512x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [27:0] sr;
  always @(posedge clk) sr <= {sr[26:0], si};
  reg [7:0] mem [0:511];
  reg [7:0] q0;
  always @(posedge clk) if (sr[17]) mem[sr[8:0]] <= sr[16:9];
  always @(posedge clk) q0 <= mem[sr[26:18]];
  always @(posedge clk) so <= ^(q0) ^ sr[27];
endmodule
