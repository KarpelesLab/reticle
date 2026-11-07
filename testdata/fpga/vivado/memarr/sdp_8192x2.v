// 8192 words of 2: one write, one read on the clock.
module sdp_8192x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [29:0] sr;
  always @(posedge clk) sr <= {sr[28:0], si};
  reg [1:0] mem [0:8191];
  reg [1:0] q0;
  always @(posedge clk) if (sr[15]) mem[sr[12:0]] <= sr[14:13];
  always @(posedge clk) q0 <= mem[sr[28:16]];
  always @(posedge clk) so <= ^(q0) ^ sr[29];
endmodule
