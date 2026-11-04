// 32 words of 2, 1 read(s), async: inferred, with initial contents.
module mem32x2init (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [12:0] sr;
  always @(posedge clk) sr <= {sr[11:0], si};
  reg [1:0] mem [0:31];
  initial begin
    mem[0] = 2'h0;
    mem[1] = 2'h1;
    mem[2] = 2'h3;
    mem[3] = 2'h1;
    mem[4] = 2'h3;
    mem[5] = 2'h0;
    mem[6] = 2'h1;
    mem[7] = 2'h0;
    mem[8] = 2'h0;
    mem[9] = 2'h0;
    mem[10] = 2'h1;
    mem[11] = 2'h0;
    mem[12] = 2'h0;
    mem[13] = 2'h1;
    mem[14] = 2'h2;
    mem[15] = 2'h2;
    mem[16] = 2'h1;
    mem[17] = 2'h1;
    mem[18] = 2'h2;
    mem[19] = 2'h2;
    mem[20] = 2'h2;
    mem[21] = 2'h2;
    mem[22] = 2'h2;
    mem[23] = 2'h2;
    mem[24] = 2'h0;
    mem[25] = 2'h2;
    mem[26] = 2'h3;
    mem[27] = 2'h2;
    mem[28] = 2'h3;
    mem[29] = 2'h0;
    mem[30] = 2'h2;
    mem[31] = 2'h0;
  end
  always @(posedge clk) if (sr[12]) mem[sr[4:0]] <= sr[6:5];
  reg [1:0] q0;
  always @(posedge clk) q0 <= mem[sr[11:7]];
  always @(posedge clk) so <= ^q0;
endmodule
