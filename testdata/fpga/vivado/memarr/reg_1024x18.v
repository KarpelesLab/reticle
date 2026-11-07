// 1024 words of 18: one write, the read registered twice.
module reg_1024x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [39:0] sr;
  always @(posedge clk) sr <= {sr[38:0], si};
  reg [17:0] mem [0:1023];
  reg [17:0] q0;
  always @(posedge clk) if (sr[28]) mem[sr[9:0]] <= sr[27:10];
  always @(posedge clk) q0 <= mem[sr[38:29]];
  reg [17:0] q1;
  always @(posedge clk) q1 <= q0;
  always @(posedge clk) so <= ^(q1) ^ sr[39];
endmodule
