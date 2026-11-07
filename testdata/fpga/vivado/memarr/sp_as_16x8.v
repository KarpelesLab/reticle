// 16 words of 8 (sp_as).
module sp_as_16x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [13:0] sr;
  always @(posedge clk) sr <= {sr[12:0], si};
  reg [7:0] mem [0:15];
  reg [7:0] q0;
  always @(posedge clk) if (sr[12]) mem[sr[3:0]] <= sr[11:4];
  always @(*) q0 = mem[sr[3:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[13];
endmodule
