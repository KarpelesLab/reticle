// 16384 words of 4: one write, one read with an enable.
module en_16384x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [34:0] sr;
  always @(posedge clk) sr <= {sr[33:0], si};
  reg [3:0] mem [0:16383];
  reg [3:0] q0;
  always @(posedge clk) if (sr[18]) mem[sr[13:0]] <= sr[17:14];
  always @(posedge clk) if (sr[33]) q0 <= mem[sr[32:19]];
  always @(posedge clk) so <= ^(q0) ^ sr[34];
endmodule
