// 1024 words of 40 (sdp), written 8 bits at a time under 5 enables, a loop over the lanes.
module bw_sdp_1024x40_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [65:0] sr;
  always @(posedge clk) sr <= {sr[64:0], si};
  reg [39:0] mem [0:1023];
  reg [39:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [39:0] dw = sr[49:10];
  wire [4:0] wew = sr[54:50];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 5; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[64:55]];
  always @(posedge clk) so <= ^((q0) & sr[39:0]) ^ sr[65];
endmodule
