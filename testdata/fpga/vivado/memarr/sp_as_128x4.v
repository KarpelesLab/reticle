// 128 words of 4 (sp_as).
module sp_as_128x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [12:0] sr;
  always @(posedge clk) sr <= {sr[11:0], si};
  reg [3:0] mem [0:127];
  reg [3:0] q0;
  always @(posedge clk) if (sr[11]) mem[sr[6:0]] <= sr[10:7];
  always @(*) q0 = mem[sr[6:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[12];
endmodule
