// 65536 words of 1 (init).
module init_65536x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [34:0] sr;
  always @(posedge clk) sr <= {sr[33:0], si};
  reg [0:0] mem [0:65535];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 65536; k = k + 1) begin
    h = {k * 64'hc54801d2168d0ca1, k * 64'h8e739512816e9367};
    mem[k] = h[127:127];
  end
  reg [0:0] q0;
  always @(posedge clk) if (sr[17]) mem[sr[15:0]] <= sr[16];
  always @(posedge clk) q0 <= mem[sr[33:18]];
  always @(posedge clk) so <= ^(q0) ^ sr[34];
endmodule
