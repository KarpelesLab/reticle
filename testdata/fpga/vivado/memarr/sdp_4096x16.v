// 4096 words of 16: one write, one read on the clock.
module sdp_4096x16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [41:0] sr;
  always @(posedge clk) sr <= {sr[40:0], si};
  reg [15:0] mem [0:4095];
  reg [15:0] q0;
  always @(posedge clk) if (sr[28]) mem[sr[11:0]] <= sr[27:12];
  always @(posedge clk) q0 <= mem[sr[40:29]];
  always @(posedge clk) so <= ^(q0) ^ sr[41];
endmodule
