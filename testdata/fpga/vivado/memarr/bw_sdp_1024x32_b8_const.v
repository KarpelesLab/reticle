// 1024 words of 32 (sdp), written 8 bits at a time under 4 enables, constant bounds.
module bw_sdp_1024x32_b8_const (
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
  integer i;
  always @(posedge clk) begin
    if (wew[0]) mem[aw][7:0] <= dw[7:0];
    if (wew[1]) mem[aw][15:8] <= dw[15:8];
    if (wew[2]) mem[aw][23:16] <= dw[23:16];
    if (wew[3]) mem[aw][31:24] <= dw[31:24];
  end
  always @(posedge clk) q0 <= mem[sr[55:46]];
  always @(posedge clk) so <= ^((q0) & sr[31:0]) ^ sr[56];
endmodule
