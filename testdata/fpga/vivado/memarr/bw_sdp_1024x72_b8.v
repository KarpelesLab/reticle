// 1024 words of 72 (sdp), written 8 bits at a time under 9 enables, a loop over the lanes.
module bw_sdp_1024x72_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [101:0] sr;
  always @(posedge clk) sr <= {sr[100:0], si};
  reg [71:0] mem [0:1023];
  reg [71:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [71:0] dw = sr[81:10];
  wire [8:0] wew = sr[90:82];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 9; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[100:91]];
  always @(posedge clk) so <= ^((q0) & sr[71:0]) ^ sr[101];
endmodule
