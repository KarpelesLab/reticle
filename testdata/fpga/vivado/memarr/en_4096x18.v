// 4096 words of 18: one write, one read with an enable.
module en_4096x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [44:0] sr;
  always @(posedge clk) sr <= {sr[43:0], si};
  reg [17:0] mem [0:4095];
  reg [17:0] q0;
  always @(posedge clk) if (sr[30]) mem[sr[11:0]] <= sr[29:12];
  always @(posedge clk) if (sr[43]) q0 <= mem[sr[42:31]];
  always @(posedge clk) so <= ^(q0) ^ sr[44];
endmodule
