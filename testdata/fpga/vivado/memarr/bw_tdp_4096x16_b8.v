// 4096 words of 16 (tdp), written 8 bits at a time under 2 enables, a loop over the lanes.
module bw_tdp_4096x16_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [60:0] sr;
  always @(posedge clk) sr <= {sr[59:0], si};
  reg [15:0] mem [0:4095];
  reg [15:0] q0;
  wire [11:0] aa = sr[11:0];
  wire [15:0] da = sr[27:12];
  wire [1:0] wea = sr[29:28];
  wire [11:0] ab = sr[41:30];
  wire [15:0] db = sr[57:42];
  wire [1:0] web = sr[59:58];
  reg [15:0] p0;
  integer i, j;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wea[i]) mem[aa][i * 8 +: 8] <= da[i * 8 +: 8];
    q0 <= mem[aa];
  end
  always @(posedge clk) begin
    for (j = 0; j < 2; j = j + 1)
      if (web[j]) mem[ab][j * 8 +: 8] <= db[j * 8 +: 8];
    p0 <= mem[ab];
  end
  always @(posedge clk) so <= ^((q0 ^ p0) & sr[15:0]) ^ sr[60];
endmodule
