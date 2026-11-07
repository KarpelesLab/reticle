// 16384 words of 2, initial contents, no write.
module rom_16384x2 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [14:0] sr;
  always @(posedge clk) sr <= {sr[13:0], si};
  reg [1:0] mem [0:16383];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 16384; k = k + 1) begin
    h = {k * 64'he447209675d14911, k * 64'hf480418f3961f7bd};
    mem[k] = h[127:126];
  end
  reg [1:0] q0;
  always @(posedge clk) q0 <= mem[sr[13:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[14];
endmodule
