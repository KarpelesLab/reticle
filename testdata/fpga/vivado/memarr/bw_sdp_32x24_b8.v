// 32 words of 24 (sdp), written 8 bits at a time under 3 enables, a loop over the lanes.
module bw_sdp_32x24_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [37:0] sr;
  always @(posedge clk) sr <= {sr[36:0], si};
  reg [23:0] mem [0:31];
  reg [23:0] q0;
  wire [4:0] aw = sr[4:0];
  wire [23:0] dw = sr[28:5];
  wire [2:0] wew = sr[31:29];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 3; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[36:32]];
  always @(posedge clk) so <= ^((q0) & sr[23:0]) ^ sr[37];
endmodule
