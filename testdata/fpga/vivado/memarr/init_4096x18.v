// 4096 words of 18 with initial contents, written and read.
module init_4096x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [43:0] sr;
  always @(posedge clk) sr <= {sr[42:0], si};
  reg [17:0] mem [0:4095];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 4096; k = k + 1) begin
    h = {k * 64'hefa46926a49c2f5f, k * 64'h7bb50b4b3e4bdf8f};
    mem[k] = h[127:110];
  end
  reg [17:0] q0;
  always @(posedge clk) if (sr[30]) mem[sr[11:0]] <= sr[29:12];
  always @(posedge clk) q0 <= mem[sr[42:31]];
  always @(posedge clk) so <= ^(q0) ^ sr[43];
endmodule
