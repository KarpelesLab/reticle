// 16384 words of 16 (sdp), written 8 bits at a time under 2 enables, a loop over the lanes.
module bw_sdp_16384x16_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [46:0] sr;
  always @(posedge clk) sr <= {sr[45:0], si};
  reg [15:0] mem [0:16383];
  reg [15:0] q0;
  wire [13:0] aw = sr[13:0];
  wire [15:0] dw = sr[29:14];
  wire [1:0] wew = sr[31:30];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[45:32]];
  always @(posedge clk) so <= ^((q0) & sr[15:0]) ^ sr[46];
endmodule
