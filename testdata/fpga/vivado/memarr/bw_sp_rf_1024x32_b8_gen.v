// 1024 words of 32 (sp_rf), written 8 bits at a time under 4 enables, a process per lane.
module bw_sp_rf_1024x32_b8_gen (
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
  genvar g;
  generate for (g = 0; g < 4; g = g + 1) begin : lane
    always @(posedge clk) if (wew[g]) mem[aw][g * 8 +: 8] <= dw[g * 8 +: 8];
  end endgenerate
  always @(posedge clk) begin
    q0 <= mem[aw];
  end
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[46];
endmodule
