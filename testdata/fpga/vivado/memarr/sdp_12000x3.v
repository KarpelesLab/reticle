// 12000 words of 3: one write, one read on the clock.
module sdp_12000x3 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [32:0] sr;
  always @(posedge clk) sr <= {sr[31:0], si};
  reg [2:0] mem [0:11999];
  reg [2:0] q0;
  always @(posedge clk) if (sr[17]) mem[sr[13:0]] <= sr[16:14];
  always @(posedge clk) q0 <= mem[sr[31:18]];
  always @(posedge clk) so <= ^(q0) ^ sr[32];
endmodule
