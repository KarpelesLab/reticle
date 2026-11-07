// 65536 words of 16 (sp_rf), written 8 bits at a time under 2 enables, a loop over the lanes.
module bw_sp_rf_65536x16_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [34:0] sr;
  always @(posedge clk) sr <= {sr[33:0], si};
  reg [15:0] mem [0:65535];
  reg [15:0] q0;
  wire [15:0] aw = sr[15:0];
  wire [15:0] dw = sr[31:16];
  wire [1:0] wew = sr[33:32];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
    q0 <= mem[aw];
  end
  always @(posedge clk) so <= ^((q0) & sr[15:0]) ^ sr[34];
endmodule
