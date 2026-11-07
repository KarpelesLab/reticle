// 16384 words of 18: one write, one read on the clock.
module sdp_16384x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [47:0] sr;
  always @(posedge clk) sr <= {sr[46:0], si};
  reg [17:0] mem [0:16383];
  reg [17:0] q0;
  always @(posedge clk) if (sr[32]) mem[sr[13:0]] <= sr[31:14];
  always @(posedge clk) q0 <= mem[sr[46:33]];
  always @(posedge clk) so <= ^(q0) ^ sr[47];
endmodule
