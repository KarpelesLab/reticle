// 1024 words of 9: one write, one read on the clock.
module sdp_1024x9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [30:0] sr;
  always @(posedge clk) sr <= {sr[29:0], si};
  reg [8:0] mem [0:1023];
  reg [8:0] q0;
  always @(posedge clk) if (sr[19]) mem[sr[9:0]] <= sr[18:10];
  always @(posedge clk) q0 <= mem[sr[29:20]];
  always @(posedge clk) so <= ^(q0) ^ sr[30];
endmodule
