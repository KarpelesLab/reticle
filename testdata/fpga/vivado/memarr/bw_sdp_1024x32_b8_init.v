// 1024 words of 32 (sdp), written 8 bits at a time under 4 enables, a loop over the lanes, initial contents.
module bw_sdp_1024x32_b8_init (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [56:0] sr;
  always @(posedge clk) sr <= {sr[55:0], si};
  reg [31:0] mem [0:1023];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 1024; k = k + 1) begin
    h = {k * 64'hba34bf7891d268b3, k * 64'h1608fd3bbc4bb383};
    mem[k] = h[127:96];
  end
  reg [31:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [31:0] dw = sr[41:10];
  wire [3:0] wew = sr[45:42];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
  end
  always @(posedge clk) q0 <= mem[sr[55:46]];
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[56];
endmodule
