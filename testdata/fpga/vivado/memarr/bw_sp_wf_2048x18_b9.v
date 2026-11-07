// 2048 words of 18 (sp_wf), written 9 bits at a time under 2 enables, a loop over the lanes.
module bw_sp_wf_2048x18_b9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [31:0] sr;
  always @(posedge clk) sr <= {sr[30:0], si};
  reg [17:0] mem [0:2047];
  reg [17:0] q0;
  wire [10:0] aw = sr[10:0];
  wire [17:0] dw = sr[28:11];
  wire [1:0] wew = sr[30:29];
  integer i;
  always @(posedge clk)
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) begin
        mem[aw][i * 9 +: 9] <= dw[i * 9 +: 9];
        q0[i * 9 +: 9] <= dw[i * 9 +: 9];
      end else q0[i * 9 +: 9] <= mem[aw][i * 9 +: 9];
  always @(posedge clk) so <= ^((q0) & sr[17:0]) ^ sr[31];
endmodule
