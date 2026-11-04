// 64 words of 3, 1 read(s), async: inferred, with initial contents.
module mem64x3init (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [15:0] sr;
  always @(posedge clk) sr <= {sr[14:0], si};
  reg [2:0] mem [0:63];
  initial begin
    mem[0] = 3'h5;
    mem[1] = 3'h4;
    mem[2] = 3'h3;
    mem[3] = 3'h1;
    mem[4] = 3'h2;
    mem[5] = 3'h4;
    mem[6] = 3'h7;
    mem[7] = 3'h0;
    mem[8] = 3'h0;
    mem[9] = 3'h5;
    mem[10] = 3'h5;
    mem[11] = 3'h5;
    mem[12] = 3'h7;
    mem[13] = 3'h2;
    mem[14] = 3'h0;
    mem[15] = 3'h2;
    mem[16] = 3'h3;
    mem[17] = 3'h3;
    mem[18] = 3'h4;
    mem[19] = 3'h4;
    mem[20] = 3'h5;
    mem[21] = 3'h2;
    mem[22] = 3'h4;
    mem[23] = 3'h7;
    mem[24] = 3'h1;
    mem[25] = 3'h5;
    mem[26] = 3'h6;
    mem[27] = 3'h0;
    mem[28] = 3'h5;
    mem[29] = 3'h1;
    mem[30] = 3'h2;
    mem[31] = 3'h5;
    mem[32] = 3'h2;
    mem[33] = 3'h4;
    mem[34] = 3'h2;
    mem[35] = 3'h0;
    mem[36] = 3'h4;
    mem[37] = 3'h5;
    mem[38] = 3'h6;
    mem[39] = 3'h3;
    mem[40] = 3'h2;
    mem[41] = 3'h7;
    mem[42] = 3'h5;
    mem[43] = 3'h4;
    mem[44] = 3'h6;
    mem[45] = 3'h3;
    mem[46] = 3'h7;
    mem[47] = 3'h7;
    mem[48] = 3'h7;
    mem[49] = 3'h4;
    mem[50] = 3'h1;
    mem[51] = 3'h2;
    mem[52] = 3'h6;
    mem[53] = 3'h5;
    mem[54] = 3'h5;
    mem[55] = 3'h1;
    mem[56] = 3'h2;
    mem[57] = 3'h1;
    mem[58] = 3'h5;
    mem[59] = 3'h7;
    mem[60] = 3'h0;
    mem[61] = 3'h7;
    mem[62] = 3'h7;
    mem[63] = 3'h3;
  end
  always @(posedge clk) if (sr[15]) mem[sr[5:0]] <= sr[8:6];
  reg [2:0] q0;
  always @(posedge clk) q0 <= mem[sr[14:9]];
  always @(posedge clk) so <= ^q0;
endmodule
