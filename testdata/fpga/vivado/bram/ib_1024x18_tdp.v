// 1024 words of 18 to infer: {'tdp': True}.
module ib_1024x18_tdp (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [58:0] sr;
  always @(posedge clk) sr <= {sr[57:0], si};
  reg [17:0] mem [0:1023];
  always @(posedge clk) if (sr[38]) mem[sr[9:0]] <= sr[27:10];
  reg [17:0] q0;
  always @(posedge clk) q0 <= mem[sr[37:28]];
  reg [17:0] p0;
  always @(posedge clk) begin if (sr[39]) mem[sr[37:28]] <= sr[57:40]; p0 <= mem[sr[9:0]]; end
  always @(posedge clk) so <= ^(q0 ^ p0) ^ sr[58];
endmodule
