// 512 words of 72 to infer: one write, one registered read.
module ib_512x72 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [91:0] sr;
  always @(posedge clk) sr <= {sr[90:0], si};
  reg [71:0] mem [0:511];
  always @(posedge clk) if (sr[90]) mem[sr[8:0]] <= sr[80:9];
  reg [71:0] q0;
  always @(posedge clk) q0 <= mem[sr[89:81]];
  always @(posedge clk) so <= ^(q0) ^ sr[91];
endmodule
