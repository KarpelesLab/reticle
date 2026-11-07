// 4096 words of 8, initial contents, no write.
module rom_4096x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [12:0] sr;
  always @(posedge clk) sr <= {sr[11:0], si};
  reg [7:0] mem [0:4095];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 4096; k = k + 1) begin
    h = {k * 64'h1def78da7d78a0af, k * 64'h37eda5b7598034a3};
    mem[k] = h[127:120];
  end
  reg [7:0] q0;
  always @(posedge clk) q0 <= mem[sr[11:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[12];
endmodule
