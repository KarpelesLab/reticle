// 512 words of 16 (sdp), written 8 bits at a time under 2 enables, a loop over the lanes.
module bw_sdp_512x16_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [36:0] sr;
  always @(posedge clk) sr <= {sr[35:0], si};
  reg [15:0] mem [0:511];
  reg [15:0] q0;
  wire [8:0] aw = sr[8:0];
  wire [15:0] dw = sr[24:9];
  wire [1:0] wew = sr[26:25];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[35:27]];
  always @(posedge clk) so <= ^((q0) & sr[15:0]) ^ sr[36];
endmodule
