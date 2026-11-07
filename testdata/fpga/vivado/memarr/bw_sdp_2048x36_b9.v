// 2048 words of 36 (sdp), written 9 bits at a time under 4 enables, a loop over the lanes.
module bw_sdp_2048x36_b9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [62:0] sr;
  always @(posedge clk) sr <= {sr[61:0], si};
  reg [35:0] mem [0:2047];
  reg [35:0] q0;
  wire [10:0] aw = sr[10:0];
  wire [35:0] dw = sr[46:11];
  wire [3:0] wew = sr[50:47];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 9 +: 9] <= dw[i * 9 +: 9];
  end
  always @(posedge clk) q0 <= mem[sr[61:51]];
  always @(posedge clk) so <= ^((q0) & sr[35:0]) ^ sr[62];
endmodule
