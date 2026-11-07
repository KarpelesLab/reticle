// 1024 words of 16: one write, one read on the clock.
module sdp_1024x16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [37:0] sr;
  always @(posedge clk) sr <= {sr[36:0], si};
  reg [15:0] mem [0:1023];
  reg [15:0] q0;
  always @(posedge clk) if (sr[26]) mem[sr[9:0]] <= sr[25:10];
  always @(posedge clk) q0 <= mem[sr[36:27]];
  always @(posedge clk) so <= ^(q0) ^ sr[37];
endmodule
