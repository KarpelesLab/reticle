// 1024 words of 8 read on no clock.
module async_1024x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [29:0] sr;
  always @(posedge clk) sr <= {sr[28:0], si};
  reg [7:0] mem [0:1023];
  reg [7:0] q0;
  always @(posedge clk) if (sr[18]) mem[sr[9:0]] <= sr[17:10];
  always @(*) q0 = mem[sr[28:19]];
  always @(posedge clk) so <= ^(q0) ^ sr[29];
endmodule
