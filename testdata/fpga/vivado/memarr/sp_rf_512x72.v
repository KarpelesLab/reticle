// 512 words of 72 (sp_rf).
module sp_rf_512x72 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [82:0] sr;
  always @(posedge clk) sr <= {sr[81:0], si};
  reg [71:0] mem [0:511];
  reg [71:0] q0;
  always @(posedge clk) begin
    if (sr[81]) mem[sr[8:0]] <= sr[80:9];
    q0 <= mem[sr[8:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[82];
endmodule
