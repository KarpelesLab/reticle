// 512 words of 36 (sdp_init), read under a mask.
module bits_sdp_init_512x36 (
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
    h = {k * 64'h39009cb5d0f42829, k * 64'h7c01d1a05f986529};
    mem[k] = h[127:92];
  end
  reg [35:0] q0;
  always @(posedge clk) if (sr[45]) mem[sr[8:0]] <= sr[44:9];
  always @(posedge clk) q0 <= mem[sr[54:46]];
  always @(posedge clk) so <= ^((q0) & sr[35:0]) ^ sr[55];
endmodule
