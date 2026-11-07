// 32 words of 32 (sp_rf).
module sp_rf_32x32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [38:0] sr;
  always @(posedge clk) sr <= {sr[37:0], si};
  reg [31:0] mem [0:31];
  reg [31:0] q0;
  always @(posedge clk) begin
    if (sr[37]) mem[sr[4:0]] <= sr[36:5];
    q0 <= mem[sr[4:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[38];
endmodule
