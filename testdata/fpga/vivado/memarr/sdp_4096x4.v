// 4096 words of 4: one write, one read on the clock.
module sdp_4096x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [29:0] sr;
  always @(posedge clk) sr <= {sr[28:0], si};
  reg [3:0] mem [0:4095];
  reg [3:0] q0;
  always @(posedge clk) if (sr[16]) mem[sr[11:0]] <= sr[15:12];
  always @(posedge clk) q0 <= mem[sr[28:17]];
  always @(posedge clk) so <= ^(q0) ^ sr[29];
endmodule
