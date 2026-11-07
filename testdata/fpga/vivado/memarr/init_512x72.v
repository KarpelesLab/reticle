// 512 words of 72 with initial contents, written and read.
module init_512x72 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [91:0] sr;
  always @(posedge clk) sr <= {sr[90:0], si};
  reg [71:0] mem [0:511];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 512; k = k + 1) begin
    h = {k * 64'h87a87d4800e65a79, k * 64'h56e3c0b5ce62c709};
    mem[k] = h[127:56];
  end
  reg [71:0] q0;
  always @(posedge clk) if (sr[81]) mem[sr[8:0]] <= sr[80:9];
  always @(posedge clk) q0 <= mem[sr[90:82]];
  always @(posedge clk) so <= ^(q0) ^ sr[91];
endmodule
