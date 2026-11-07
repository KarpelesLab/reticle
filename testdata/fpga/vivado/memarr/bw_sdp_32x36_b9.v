// 32 words of 36 (sdp), written 9 bits at a time under 4 enables, a loop over the lanes.
module bw_sdp_32x36_b9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [50:0] sr;
  always @(posedge clk) sr <= {sr[49:0], si};
  reg [35:0] mem [0:31];
  reg [35:0] q0;
  wire [4:0] aw = sr[4:0];
  wire [35:0] dw = sr[40:5];
  wire [3:0] wew = sr[44:41];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 9 +: 9] <= dw[i * 9 +: 9];
  end
  always @(posedge clk) q0 <= mem[sr[49:45]];
  always @(posedge clk) so <= ^((q0) & sr[35:0]) ^ sr[50];
endmodule
