// 1024 words of 18: one write, one read with an enable.
module en_1024x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [40:0] sr;
  always @(posedge clk) sr <= {sr[39:0], si};
  reg [17:0] mem [0:1023];
  reg [17:0] q0;
  always @(posedge clk) if (sr[28]) mem[sr[9:0]] <= sr[27:10];
  always @(posedge clk) if (sr[39]) q0 <= mem[sr[38:29]];
  always @(posedge clk) so <= ^(q0) ^ sr[40];
endmodule
