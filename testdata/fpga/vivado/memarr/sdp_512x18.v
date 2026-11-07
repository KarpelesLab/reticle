// 512 words of 18: one write, one read on the clock.
module sdp_512x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [37:0] sr;
  always @(posedge clk) sr <= {sr[36:0], si};
  reg [17:0] mem [0:511];
  reg [17:0] q0;
  always @(posedge clk) if (sr[27]) mem[sr[8:0]] <= sr[26:9];
  always @(posedge clk) q0 <= mem[sr[36:28]];
  always @(posedge clk) so <= ^(q0) ^ sr[37];
endmodule
