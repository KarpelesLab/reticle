// 1024 words of 16 (sdp), written 4 bits at a time under 4 enables, a loop over the lanes.
module bw_sdp_1024x16_b4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [40:0] sr;
  always @(posedge clk) sr <= {sr[39:0], si};
  reg [15:0] mem [0:1023];
  reg [15:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [15:0] dw = sr[25:10];
  wire [3:0] wew = sr[29:26];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 4 +: 4] <= dw[i * 4 +: 4];
  end
  always @(posedge clk) q0 <= mem[sr[39:30]];
  always @(posedge clk) so <= ^((q0) & sr[15:0]) ^ sr[40];
endmodule
