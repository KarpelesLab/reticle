// 1000 words of 18: one write, one read on the clock.
module sdp_1000x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [39:0] sr;
  always @(posedge clk) sr <= {sr[38:0], si};
  reg [17:0] mem [0:999];
  reg [17:0] q0;
  always @(posedge clk) if (sr[28]) mem[sr[9:0]] <= sr[27:10];
  always @(posedge clk) q0 <= mem[sr[38:29]];
  always @(posedge clk) so <= ^(q0) ^ sr[39];
endmodule
