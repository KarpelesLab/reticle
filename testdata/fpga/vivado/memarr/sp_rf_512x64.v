// 512 words of 64 (sp_rf).
module sp_rf_512x64 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [74:0] sr;
  always @(posedge clk) sr <= {sr[73:0], si};
  reg [63:0] mem [0:511];
  reg [63:0] q0;
  always @(posedge clk) begin
    if (sr[73]) mem[sr[8:0]] <= sr[72:9];
    q0 <= mem[sr[8:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[74];
endmodule
