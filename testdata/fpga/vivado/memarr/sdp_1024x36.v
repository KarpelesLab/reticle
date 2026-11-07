// 1024 words of 36: one write, one read on the clock.
module sdp_1024x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [57:0] sr;
  always @(posedge clk) sr <= {sr[56:0], si};
  reg [35:0] mem [0:1023];
  reg [35:0] q0;
  always @(posedge clk) if (sr[46]) mem[sr[9:0]] <= sr[45:10];
  always @(posedge clk) q0 <= mem[sr[56:47]];
  always @(posedge clk) so <= ^(q0) ^ sr[57];
endmodule
