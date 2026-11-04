// 1024 words of 18 to infer: one write, one registered read.
module ib_1024x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [39:0] sr;
  always @(posedge clk) sr <= {sr[38:0], si};
  reg [17:0] mem [0:1023];
  always @(posedge clk) if (sr[38]) mem[sr[9:0]] <= sr[27:10];
  reg [17:0] q0;
  always @(posedge clk) q0 <= mem[sr[37:28]];
  always @(posedge clk) so <= ^(q0) ^ sr[39];
endmodule
