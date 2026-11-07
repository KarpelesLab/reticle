// 512 words of 72 (sdp), written 9 bits at a time under 8 enables, a loop over the lanes, initial contents.
module bw_sdp_512x72_b9_init (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [98:0] sr;
  always @(posedge clk) sr <= {sr[97:0], si};
  reg [71:0] mem [0:511];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 512; k = k + 1) begin
    h = {k * 64'h1f96bf6ceaa32f2b, k * 64'hae2278d824756a79};
    mem[k] = h[127:56];
  end
  reg [71:0] q0;
  wire [8:0] aw = sr[8:0];
  wire [71:0] dw = sr[80:9];
  wire [7:0] wew = sr[88:81];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 8; i = i + 1)
      if (wew[i]) mem[aw][i * 9 +: 9] <= dw[i * 9 +: 9];
  end
  always @(posedge clk) q0 <= mem[sr[97:89]];
  always @(posedge clk) so <= ^((q0) & sr[71:0]) ^ sr[98];
endmodule
