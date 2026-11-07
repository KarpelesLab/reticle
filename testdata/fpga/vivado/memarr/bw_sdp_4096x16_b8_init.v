// 4096 words of 16 (sdp), written 8 bits at a time under 2 enables, a loop over the lanes, initial contents.
module bw_sdp_4096x16_b8_init (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [42:0] sr;
  always @(posedge clk) sr <= {sr[41:0], si};
  reg [15:0] mem [0:4095];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 4096; k = k + 1) begin
    h = {k * 64'ha6a9d1052a2ff15f, k * 64'h0589bb42bacad48b};
    mem[k] = h[127:112];
  end
  reg [15:0] q0;
  wire [11:0] aw = sr[11:0];
  wire [15:0] dw = sr[27:12];
  wire [1:0] wew = sr[29:28];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[41:30]];
  always @(posedge clk) so <= ^((q0) & sr[15:0]) ^ sr[42];
endmodule
