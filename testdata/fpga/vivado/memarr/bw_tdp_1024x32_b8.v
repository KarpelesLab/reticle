// 1024 words of 32 (tdp), written 8 bits at a time under 4 enables, a loop over the lanes.
module bw_tdp_1024x32_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [92:0] sr;
  always @(posedge clk) sr <= {sr[91:0], si};
  reg [31:0] mem [0:1023];
  reg [31:0] q0;
  wire [9:0] aa = sr[9:0];
  wire [31:0] da = sr[41:10];
  wire [3:0] wea = sr[45:42];
  wire [9:0] ab = sr[55:46];
  wire [31:0] db = sr[87:56];
  wire [3:0] web = sr[91:88];
  reg [31:0] p0;
  integer i, j;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wea[i]) mem[aa][i * 8 +: 8] <= da[i * 8 +: 8];
    q0 <= mem[aa];
  end
  always @(posedge clk) begin
    for (j = 0; j < 4; j = j + 1)
      if (web[j]) mem[ab][j * 8 +: 8] <= db[j * 8 +: 8];
    p0 <= mem[ab];
  end
  always @(posedge clk) so <= ^((q0 ^ p0) & sr[31:0]) ^ sr[92];
endmodule
