// 512 words of 36 to infer: one write, one registered read.
module ib_512x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [55:0] sr;
  always @(posedge clk) sr <= {sr[54:0], si};
  reg [35:0] mem [0:511];
  always @(posedge clk) if (sr[54]) mem[sr[8:0]] <= sr[44:9];
  reg [35:0] q0;
  always @(posedge clk) q0 <= mem[sr[53:45]];
  always @(posedge clk) so <= ^(q0) ^ sr[55];
endmodule
