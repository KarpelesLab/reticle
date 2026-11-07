// 512 words of 64: one write, one read on the clock.
module sdp_512x64 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [83:0] sr;
  always @(posedge clk) sr <= {sr[82:0], si};
  reg [63:0] mem [0:511];
  reg [63:0] q0;
  always @(posedge clk) if (sr[73]) mem[sr[8:0]] <= sr[72:9];
  always @(posedge clk) q0 <= mem[sr[82:74]];
  always @(posedge clk) so <= ^(q0) ^ sr[83];
endmodule
