// 32 words of 32 (sdp), written 8 bits at a time under 4 enables, a loop over the lanes.
module bw_sdp_32x32_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [46:0] sr;
  always @(posedge clk) sr <= {sr[45:0], si};
  reg [31:0] mem [0:31];
  reg [31:0] q0;
  wire [4:0] aw = sr[4:0];
  wire [31:0] dw = sr[36:5];
  wire [3:0] wew = sr[40:37];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[45:41]];
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[46];
endmodule
