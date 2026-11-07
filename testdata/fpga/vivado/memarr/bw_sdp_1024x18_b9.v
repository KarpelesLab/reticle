// 1024 words of 18 (sdp), written 9 bits at a time under 2 enables, a loop over the lanes.
module bw_sdp_1024x18_b9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [40:0] sr;
  always @(posedge clk) sr <= {sr[39:0], si};
  reg [17:0] mem [0:1023];
  reg [17:0] q0;
  wire [9:0] aw = sr[9:0];
  wire [17:0] dw = sr[27:10];
  wire [1:0] wew = sr[29:28];
  integer i;
  always @(posedge clk) begin
    for (i = 0; i < 2; i = i + 1)
      if (wew[i]) mem[aw][i * 9 +: 9] <= dw[i * 9 +: 9];
  end
  always @(posedge clk) q0 <= mem[sr[39:30]];
  always @(posedge clk) so <= ^((q0) & sr[17:0]) ^ sr[40];
endmodule
