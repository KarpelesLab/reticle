// 32 words of 6, 1 read(s), async: inferred, with initial contents.
module mem32x6init (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [16:0] sr;
  always @(posedge clk) sr <= {sr[15:0], si};
  reg [5:0] mem [0:31];
  initial begin
    mem[0] = 6'h6;
    mem[1] = 6'h0;
    mem[2] = 6'h8;
    mem[3] = 6'h2c;
    mem[4] = 6'h35;
    mem[5] = 6'h1;
    mem[6] = 6'h27;
    mem[7] = 6'h8;
    mem[8] = 6'h1e;
    mem[9] = 6'h9;
    mem[10] = 6'h2f;
    mem[11] = 6'h3a;
    mem[12] = 6'h24;
    mem[13] = 6'h0;
    mem[14] = 6'h3e;
    mem[15] = 6'h11;
    mem[16] = 6'h17;
    mem[17] = 6'h0;
    mem[18] = 6'h33;
    mem[19] = 6'h18;
    mem[20] = 6'h1d;
    mem[21] = 6'h35;
    mem[22] = 6'h3b;
    mem[23] = 6'h10;
    mem[24] = 6'h20;
    mem[25] = 6'h2;
    mem[26] = 6'he;
    mem[27] = 6'h39;
    mem[28] = 6'h0;
    mem[29] = 6'h27;
    mem[30] = 6'h27;
    mem[31] = 6'h31;
  end
  always @(posedge clk) if (sr[16]) mem[sr[4:0]] <= sr[10:5];
  reg [5:0] q0;
  always @(posedge clk) q0 <= mem[sr[15:11]];
  always @(posedge clk) so <= ^q0;
endmodule
