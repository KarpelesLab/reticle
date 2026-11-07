// 1024 words of 32 (sp_rf), written 8 bits at a time under 4 enables, a loop over the lanes.
module bw_sp_rf_1024x32_b8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [46:0] sr;
  always @(posedge clk) sr <= {sr[45:0], si};
  reg [31:0] mem [0:1023];
  reg [31:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [31:0] dw = sr[41:10];
  wire [3:0] wew = sr[45:42];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 8 +: 8] <= dw[i * 8 +: 8];
    q0 <= mem[aw];
  end
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[46];
endmodule
