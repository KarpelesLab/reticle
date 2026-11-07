// 65536 words of 1 (tdp).
module tdp_65536x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [36:0] sr;
  always @(posedge clk) sr <= {sr[35:0], si};
  reg [0:0] mem [0:65535];
  reg [0:0] q0;
  reg [0:0] p0;
  always @(posedge clk) begin
    if (sr[17]) mem[sr[15:0]] <= sr[16];
    q0 <= mem[sr[15:0]];
  end
  always @(posedge clk) begin
    if (sr[35]) mem[sr[33:18]] <= sr[34];
    p0 <= mem[sr[33:18]];
  end
  always @(posedge clk) so <= ^(q0 ^ p0) ^ sr[36];
endmodule
