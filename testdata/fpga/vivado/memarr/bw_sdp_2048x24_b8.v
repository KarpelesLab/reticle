// 2048 words of 24 (sdp), written 8 bits at a time under 3 enables, a loop over the lanes.
module bw_sdp_2048x24_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [49:0] sr;
  always @(posedge clk) sr <= {sr[48:0], si};
  reg [23:0] mem [0:2047];
  reg [23:0] q0;
  wire [10:0] aw = sr[10:0];
  wire [23:0] dw = sr[34:11];
  wire [2:0] wew = sr[37:35];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 3; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[48:38]];
  always @(posedge clk) so <= ^((q0) & sr[23:0]) ^ sr[49];
endmodule
