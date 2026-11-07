// 16384 words of 4: two addresses each read and written.
module tdp_16384x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [38:0] sr;
  always @(posedge clk) sr <= {sr[37:0], si};
  reg [3:0] mem [0:16383];
  reg [3:0] q0;
  reg [3:0] p0;
  always @(posedge clk) begin
    if (sr[18]) mem[sr[13:0]] <= sr[17:14];
    q0 <= mem[sr[13:0]];
  end
  always @(posedge clk) begin
    if (sr[37]) mem[sr[32:19]] <= sr[36:33];
    p0 <= mem[sr[32:19]];
  end
  always @(posedge clk) so <= ^(q0 ^ p0) ^ sr[38];
endmodule
