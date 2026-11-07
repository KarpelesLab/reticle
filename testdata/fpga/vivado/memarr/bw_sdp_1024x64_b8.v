// 1024 words of 64 (sdp), written 8 bits at a time under 8 enables, a loop over the lanes.
module bw_sdp_1024x64_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [92:0] sr;
  always @(posedge clk) sr <= {sr[91:0], si};
  reg [63:0] mem [0:1023];
  reg [63:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [63:0] dw = sr[73:10];
  wire [7:0] wew = sr[81:74];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 8; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[91:82]];
  always @(posedge clk) so <= ^((q0) & sr[63:0]) ^ sr[92];
endmodule
