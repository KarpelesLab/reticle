// 1024 words of 32: one write, one read on the clock.
module sdp_1024x32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [53:0] sr;
  always @(posedge clk) sr <= {sr[52:0], si};
  reg [31:0] mem [0:1023];
  reg [31:0] q0;
  always @(posedge clk) if (sr[42]) mem[sr[9:0]] <= sr[41:10];
  always @(posedge clk) q0 <= mem[sr[52:43]];
  always @(posedge clk) so <= ^(q0) ^ sr[53];
endmodule
