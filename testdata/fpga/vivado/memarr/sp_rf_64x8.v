// 64 words of 8 (sp_rf).
module sp_rf_64x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [15:0] sr;
  always @(posedge clk) sr <= {sr[14:0], si};
  reg [7:0] mem [0:63];
  reg [7:0] q0;
  always @(posedge clk) begin
    if (sr[14]) mem[sr[5:0]] <= sr[13:6];
    q0 <= mem[sr[5:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[15];
endmodule
