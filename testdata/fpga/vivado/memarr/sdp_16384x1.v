// 16384 words of 1: one write, one read on the clock.
module sdp_16384x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [30:0] sr;
  always @(posedge clk) sr <= {sr[29:0], si};
  reg [0:0] mem [0:16383];
  reg [0:0] q0;
  always @(posedge clk) if (sr[15]) mem[sr[13:0]] <= sr[14];
  always @(posedge clk) q0 <= mem[sr[29:16]];
  always @(posedge clk) so <= ^(q0) ^ sr[30];
endmodule
