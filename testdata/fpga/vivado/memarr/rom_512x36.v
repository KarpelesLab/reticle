// 512 words of 36, initial contents, no write.
module rom_512x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  reg [35:0] mem [0:511];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 512; k = k + 1) begin
    h = {k * 64'h0d99e56b71cdc455, k * 64'hfbe4ce7835f4941d};
    mem[k] = h[127:92];
  end
  reg [35:0] q0;
  always @(posedge clk) q0 <= mem[sr[8:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[9];
endmodule
