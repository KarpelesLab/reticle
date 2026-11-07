// 1024 words of 16 (sdp), written 8 bits at a time under 2 enables, a loop over the lanes.
module bw_sdp_1024x16_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [38:0] sr;
  always @(posedge clk) sr <= {sr[37:0], si};
  reg [15:0] mem [0:1023];
  reg [15:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [15:0] dw = sr[25:10];
  wire [1:0] wew = sr[27:26];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[37:28]];
  always @(posedge clk) so <= ^((q0) & sr[15:0]) ^ sr[38];
endmodule
