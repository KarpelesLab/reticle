// 1024 words of 72: one write, one read on the clock.
module sdp_1024x72 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [93:0] sr;
  always @(posedge clk) sr <= {sr[92:0], si};
  reg [71:0] mem [0:1023];
  reg [71:0] q0;
  always @(posedge clk) if (sr[82]) mem[sr[9:0]] <= sr[81:10];
  always @(posedge clk) q0 <= mem[sr[92:83]];
  always @(posedge clk) so <= ^(q0) ^ sr[93];
endmodule
