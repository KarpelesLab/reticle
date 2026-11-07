// 1024 words of 1: one write, one read on the clock.
module sdp_1024x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [22:0] sr;
  always @(posedge clk) sr <= {sr[21:0], si};
  reg [0:0] mem [0:1023];
  reg [0:0] q0;
  always @(posedge clk) if (sr[11]) mem[sr[9:0]] <= sr[10];
  always @(posedge clk) q0 <= mem[sr[21:12]];
  always @(posedge clk) so <= ^(q0) ^ sr[22];
endmodule
