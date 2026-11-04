// 32 words of 2, 1 read(s), async: inferred.
module mem32x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [12:0] sr;
  always @(posedge clk) sr <= {sr[11:0], si};
  reg [1:0] mem [0:31];
  always @(posedge clk) if (sr[12]) mem[sr[4:0]] <= sr[6:5];
  reg [1:0] q0;
  always @(posedge clk) q0 <= mem[sr[11:7]];
  always @(posedge clk) so <= ^q0;
endmodule
