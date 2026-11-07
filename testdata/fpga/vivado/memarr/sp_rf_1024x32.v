// 1024 words of 32 (sp_rf).
module sp_rf_1024x32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [43:0] sr;
  always @(posedge clk) sr <= {sr[42:0], si};
  reg [31:0] mem [0:1023];
  reg [31:0] q0;
  always @(posedge clk) begin
    if (sr[42]) mem[sr[9:0]] <= sr[41:10];
    q0 <= mem[sr[9:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[43];
endmodule
