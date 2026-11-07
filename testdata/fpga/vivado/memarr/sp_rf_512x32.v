// 512 words of 32 (sp_rf).
module sp_rf_512x32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [42:0] sr;
  always @(posedge clk) sr <= {sr[41:0], si};
  reg [31:0] mem [0:511];
  reg [31:0] q0;
  always @(posedge clk) begin
    if (sr[41]) mem[sr[8:0]] <= sr[40:9];
    q0 <= mem[sr[8:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[42];
endmodule
