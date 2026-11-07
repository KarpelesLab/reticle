// 2048 words of 18 with initial contents, written and read.
module init_2048x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [41:0] sr;
  always @(posedge clk) sr <= {sr[40:0], si};
  reg [17:0] mem [0:2047];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 2048; k = k + 1) begin
    h = {k * 64'h22ce8f387a0c8c01, k * 64'hd28d8f9ea9fbe58b};
    mem[k] = h[127:110];
  end
  reg [17:0] q0;
  always @(posedge clk) if (sr[29]) mem[sr[10:0]] <= sr[28:11];
  always @(posedge clk) q0 <= mem[sr[40:30]];
  always @(posedge clk) so <= ^(q0) ^ sr[41];
endmodule
