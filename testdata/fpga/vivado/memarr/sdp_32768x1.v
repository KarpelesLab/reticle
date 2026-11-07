// 32768 words of 1: one write, one read on the clock.
module sdp_32768x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [32:0] sr;
  always @(posedge clk) sr <= {sr[31:0], si};
  reg [0:0] mem [0:32767];
  reg [0:0] q0;
  always @(posedge clk) if (sr[16]) mem[sr[14:0]] <= sr[15];
  always @(posedge clk) q0 <= mem[sr[31:17]];
  always @(posedge clk) so <= ^(q0) ^ sr[32];
endmodule
