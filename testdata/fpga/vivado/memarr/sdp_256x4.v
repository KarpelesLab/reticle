// 256 words of 4: one write, one read on the clock.
module sdp_256x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [21:0] sr;
  always @(posedge clk) sr <= {sr[20:0], si};
  reg [3:0] mem [0:255];
  reg [3:0] q0;
  always @(posedge clk) if (sr[12]) mem[sr[7:0]] <= sr[11:8];
  always @(posedge clk) q0 <= mem[sr[20:13]];
  always @(posedge clk) so <= ^(q0) ^ sr[21];
endmodule
