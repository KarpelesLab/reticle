// 512 words of 36 (sp_nc).
module sp_nc_512x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [46:0] sr;
  always @(posedge clk) sr <= {sr[45:0], si};
  reg [35:0] mem [0:511];
  reg [35:0] q0;
  always @(posedge clk) begin
    if (sr[45]) mem[sr[8:0]] <= sr[44:9];
    else q0 <= mem[sr[8:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[46];
endmodule
