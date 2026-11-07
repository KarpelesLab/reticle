// 1024 words of 36 (sp_rf), written 9 bits at a time under 4 enables, a loop over the lanes, initial contents.
module bw_sp_rf_1024x36_b9_init (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [50:0] sr;
  always @(posedge clk) sr <= {sr[49:0], si};
  reg [35:0] mem [0:1023];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 1024; k = k + 1) begin
    h = {k * 64'h345fd1ec98e614c3, k * 64'heac55dd39a3be1eb};
    mem[k] = h[127:92];
  end
  reg [35:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [35:0] dw = sr[45:10];
  wire [3:0] wew = sr[49:46];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 4; i = i + 1)
      if (wew[i]) mem[aw][i * 9 +: 9] <= dw[i * 9 +: 9];
    q0 <= mem[aw];
  end
  always @(posedge clk) so <= ^((q0) & sr[35:0]) ^ sr[50];
endmodule
