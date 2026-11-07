// 16384 words of 2: one write, one read on the clock.
module sdp_16384x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [31:0] sr;
  always @(posedge clk) sr <= {sr[30:0], si};
  reg [1:0] mem [0:16383];
  reg [1:0] q0;
  always @(posedge clk) if (sr[16]) mem[sr[13:0]] <= sr[15:14];
  always @(posedge clk) q0 <= mem[sr[30:17]];
  always @(posedge clk) so <= ^(q0) ^ sr[31];
endmodule
