// 256 words of 24 (sp_rf).
module sp_rf_256x24 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [33:0] sr;
  always @(posedge clk) sr <= {sr[32:0], si};
  reg [23:0] mem [0:255];
  reg [23:0] q0;
  always @(posedge clk) begin
    if (sr[32]) mem[sr[7:0]] <= sr[31:8];
    q0 <= mem[sr[7:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[33];
endmodule
