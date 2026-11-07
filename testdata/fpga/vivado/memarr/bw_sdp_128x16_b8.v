// 128 words of 16 (sdp), written 8 bits at a time under 2 enables, a loop over the lanes.
module bw_sdp_128x16_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [32:0] sr;
  always @(posedge clk) sr <= {sr[31:0], si};
  reg [15:0] mem [0:127];
  reg [15:0] q0;
  wire [6:0] aw = sr[6:0];
  wire [15:0] dw = sr[22:7];
  wire [1:0] wew = sr[24:23];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[31:25]];
  always @(posedge clk) so <= ^((q0) & sr[15:0]) ^ sr[32];
endmodule
