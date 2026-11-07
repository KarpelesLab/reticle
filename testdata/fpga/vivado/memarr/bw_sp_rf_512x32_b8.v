// 512 words of 32 (sp_rf), written 8 bits at a time under 4 enables, a loop over the lanes.
module bw_sp_rf_512x32_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [45:0] sr;
  always @(posedge clk) sr <= {sr[44:0], si};
  reg [31:0] mem [0:511];
  reg [31:0] q0;
  wire [8:0] aw = sr[8:0];
  wire [31:0] dw = sr[40:9];
  wire [3:0] wew = sr[44:41];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
    q0 <= mem[aw];
  end
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[45];
endmodule
