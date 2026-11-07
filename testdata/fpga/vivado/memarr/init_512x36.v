// 512 words of 36 with initial contents, written and read.
module init_512x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [55:0] sr;
  always @(posedge clk) sr <= {sr[54:0], si};
  reg [35:0] mem [0:511];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 512; k = k + 1) begin
    h = {k * 64'hbd3f19fd7d67a83d, k * 64'hd2627c03f89d211d};
    mem[k] = h[127:92];
  end
  reg [35:0] q0;
  always @(posedge clk) if (sr[45]) mem[sr[8:0]] <= sr[44:9];
  always @(posedge clk) q0 <= mem[sr[54:46]];
  always @(posedge clk) so <= ^(q0) ^ sr[55];
endmodule
