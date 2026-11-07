// 512 words of 1 (sp_as).
module sp_as_512x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [11:0] sr;
  always @(posedge clk) sr <= {sr[10:0], si};
  reg [0:0] mem [0:511];
  reg [0:0] q0;
  always @(posedge clk) if (sr[10]) mem[sr[8:0]] <= sr[9];
  always @(*) q0 = mem[sr[8:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[11];
endmodule
