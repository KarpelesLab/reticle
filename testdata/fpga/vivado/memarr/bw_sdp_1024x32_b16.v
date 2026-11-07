// 1024 words of 32 (sdp), written 16 bits at a time under 2 enables, a loop over the lanes.
module bw_sdp_1024x32_b16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [54:0] sr;
  always @(posedge clk) sr <= {sr[53:0], si};
  reg [31:0] mem [0:1023];
  reg [31:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [31:0] dw = sr[41:10];
  wire [1:0] wew = sr[43:42];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) mem[aw][i * 16 +: 16] <= dw[i * 16 +: 16];
  end
  always @(posedge clk) q0 <= mem[sr[53:44]];
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[54];
endmodule
