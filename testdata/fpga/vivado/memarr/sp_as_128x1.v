// 128 words of 1 (sp_as).
module sp_as_128x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  reg [0:0] mem [0:127];
  reg [0:0] q0;
  always @(posedge clk) if (sr[8]) mem[sr[6:0]] <= sr[7];
  always @(*) q0 = mem[sr[6:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[9];
endmodule
