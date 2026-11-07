// 512 words of 36: one write, one read on the clock.
module sdp_512x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [55:0] sr;
  always @(posedge clk) sr <= {sr[54:0], si};
  reg [35:0] mem [0:511];
  reg [35:0] q0;
  always @(posedge clk) if (sr[45]) mem[sr[8:0]] <= sr[44:9];
  always @(posedge clk) q0 <= mem[sr[54:46]];
  always @(posedge clk) so <= ^(q0) ^ sr[55];
endmodule
