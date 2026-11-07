// 6000 words of 6: one write, one read on the clock.
module sdp_6000x6 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [33:0] sr;
  always @(posedge clk) sr <= {sr[32:0], si};
  reg [5:0] mem [0:5999];
  reg [5:0] q0;
  always @(posedge clk) if (sr[19]) mem[sr[12:0]] <= sr[18:13];
  always @(posedge clk) q0 <= mem[sr[32:20]];
  always @(posedge clk) so <= ^(q0) ^ sr[33];
endmodule
