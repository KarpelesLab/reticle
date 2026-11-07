// 512 words of 18 (sp_rf).
module sp_rf_512x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [28:0] sr;
  always @(posedge clk) sr <= {sr[27:0], si};
  reg [17:0] mem [0:511];
  reg [17:0] q0;
  always @(posedge clk) begin
    if (sr[27]) mem[sr[8:0]] <= sr[26:9];
    q0 <= mem[sr[8:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[28];
endmodule
