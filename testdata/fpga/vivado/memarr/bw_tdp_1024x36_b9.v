// 1024 words of 36 (tdp), written 9 bits at a time under 4 enables, a loop over the lanes.
module bw_tdp_1024x36_b9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [100:0] sr;
  always @(posedge clk) sr <= {sr[99:0], si};
  reg [35:0] mem [0:1023];
  reg [35:0] q0;
  wire [9:0] aa = sr[9:0];
  wire [35:0] da = sr[45:10];
  wire [3:0] wea = sr[49:46];
  wire [9:0] ab = sr[59:50];
  wire [35:0] db = sr[95:60];
  wire [3:0] web = sr[99:96];
  reg [35:0] p0;
  integer i, j;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wea[i]) mem[aa][i * 9 +: 9] <= da[i * 9 +: 9];
    q0 <= mem[aa];
  end
  always @(posedge clk) begin
    for (j = 0; j < 4; j = j + 1)
      if (web[j]) mem[ab][j * 9 +: 9] <= db[j * 9 +: 9];
    p0 <= mem[ab];
  end
  always @(posedge clk) so <= ^((q0 ^ p0) & sr[35:0]) ^ sr[100];
endmodule
