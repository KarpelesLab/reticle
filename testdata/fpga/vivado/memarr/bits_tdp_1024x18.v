// 1024 words of 18 (tdp), read under a mask.
module bits_tdp_1024x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [58:0] sr;
  always @(posedge clk) sr <= {sr[57:0], si};
  reg [17:0] mem [0:1023];
  reg [17:0] q0;
  reg [17:0] p0;
  always @(posedge clk) begin
    if (sr[28]) mem[sr[9:0]] <= sr[27:10];
    q0 <= mem[sr[9:0]];
  end
  always @(posedge clk) begin
    if (sr[57]) mem[sr[38:29]] <= sr[56:39];
    p0 <= mem[sr[38:29]];
  end
  always @(posedge clk) so <= ^((q0 ^ p0) & sr[17:0]) ^ sr[58];
endmodule
