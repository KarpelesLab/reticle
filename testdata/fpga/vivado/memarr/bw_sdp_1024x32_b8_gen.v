// 1024 words of 32 (sdp), written 8 bits at a time under 4 enables, a process per lane.
module bw_sdp_1024x32_b8_gen (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [56:0] sr;
  always @(posedge clk) sr <= {sr[55:0], si};
  reg [31:0] mem [0:1023];
  reg [31:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [31:0] dw = sr[41:10];
  wire [3:0] wew = sr[45:42];
  genvar g;
  generate for (g = 0; g < 4; g = g + 1) begin : lane
    always @(posedge clk) if (wew[g]) mem[aw][g * 8 +: 8] <= dw[g * 8 +: 8];
  end endgenerate
  always @(posedge clk) q0 <= mem[sr[55:46]];
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[56];
endmodule
