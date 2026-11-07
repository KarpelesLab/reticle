// 20000 words of 2: one write, one read on the clock.
module sdp_20000x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [33:0] sr;
  always @(posedge clk) sr <= {sr[32:0], si};
  reg [1:0] mem [0:19999];
  reg [1:0] q0;
  always @(posedge clk) if (sr[17]) mem[sr[14:0]] <= sr[16:15];
  always @(posedge clk) q0 <= mem[sr[32:18]];
  always @(posedge clk) so <= ^(q0) ^ sr[33];
endmodule
