// 16384 words of 4: one write, the read registered twice.
module reg_16384x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [33:0] sr;
  always @(posedge clk) sr <= {sr[32:0], si};
  reg [3:0] mem [0:16383];
  reg [3:0] q0;
  always @(posedge clk) if (sr[18]) mem[sr[13:0]] <= sr[17:14];
  always @(posedge clk) q0 <= mem[sr[32:19]];
  reg [3:0] q1;
  always @(posedge clk) q1 <= q0;
  always @(posedge clk) so <= ^(q1) ^ sr[33];
endmodule
