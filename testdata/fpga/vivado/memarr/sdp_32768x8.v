// 32768 words of 8: one write, one read on the clock.
module sdp_32768x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [39:0] sr;
  always @(posedge clk) sr <= {sr[38:0], si};
  reg [7:0] mem [0:32767];
  reg [7:0] q0;
  always @(posedge clk) if (sr[23]) mem[sr[14:0]] <= sr[22:15];
  always @(posedge clk) q0 <= mem[sr[38:24]];
  always @(posedge clk) so <= ^(q0) ^ sr[39];
endmodule
