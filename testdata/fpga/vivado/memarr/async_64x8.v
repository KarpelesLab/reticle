// 64 words of 8 read on no clock.
module async_64x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [21:0] sr;
  always @(posedge clk) sr <= {sr[20:0], si};
  reg [7:0] mem [0:63];
  reg [7:0] q0;
  always @(posedge clk) if (sr[14]) mem[sr[5:0]] <= sr[13:6];
  always @(*) q0 = mem[sr[20:15]];
  always @(posedge clk) so <= ^(q0) ^ sr[21];
endmodule
