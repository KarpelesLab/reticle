// 4096 words of 18: one write, the read registered twice.
module reg_4096x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [43:0] sr;
  always @(posedge clk) sr <= {sr[42:0], si};
  reg [17:0] mem [0:4095];
  reg [17:0] q0;
  always @(posedge clk) if (sr[30]) mem[sr[11:0]] <= sr[29:12];
  always @(posedge clk) q0 <= mem[sr[42:31]];
  reg [17:0] q1;
  always @(posedge clk) q1 <= q0;
  always @(posedge clk) so <= ^(q1) ^ sr[43];
endmodule
