// 1024 words of 20: one write, one read on the clock.
module sdp_1024x20 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [41:0] sr;
  always @(posedge clk) sr <= {sr[40:0], si};
  reg [19:0] mem [0:1023];
  reg [19:0] q0;
  always @(posedge clk) if (sr[30]) mem[sr[9:0]] <= sr[29:10];
  always @(posedge clk) q0 <= mem[sr[40:31]];
  always @(posedge clk) so <= ^(q0) ^ sr[41];
endmodule
