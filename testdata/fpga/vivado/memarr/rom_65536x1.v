// 65536 words of 1 (rom).
module rom_65536x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [16:0] sr;
  always @(posedge clk) sr <= {sr[15:0], si};
  reg [0:0] mem [0:65535];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 65536; k = k + 1) begin
    h = {k * 64'h55573a333489ae4d, k * 64'hcb2e0ff2014a0961};
    mem[k] = h[127:127];
  end
  reg [0:0] q0;
  always @(posedge clk) q0 <= mem[sr[15:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[16];
endmodule
