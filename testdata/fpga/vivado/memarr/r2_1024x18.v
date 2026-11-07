// 1024 words of 18: one write and two reads.
module r2_1024x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [49:0] sr;
  always @(posedge clk) sr <= {sr[48:0], si};
  reg [17:0] mem [0:1023];
  reg [17:0] q0;
  always @(posedge clk) if (sr[28]) mem[sr[9:0]] <= sr[27:10];
  always @(posedge clk) q0 <= mem[sr[38:29]];
  reg [17:0] p0;
  always @(posedge clk) p0 <= mem[sr[48:39]];
  always @(posedge clk) so <= ^(q0 ^ p0) ^ sr[49];
endmodule
