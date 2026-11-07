// 16 words of 32 (sdp), written 8 bits at a time under 4 enables, a loop over the lanes.
module bw_sdp_16x32_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [44:0] sr;
  always @(posedge clk) sr <= {sr[43:0], si};
  reg [31:0] mem [0:15];
  reg [31:0] q0;
  wire [3:0] aw = sr[3:0];
  wire [31:0] dw = sr[35:4];
  wire [3:0] wew = sr[39:36];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[43:40]];
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[44];
endmodule
