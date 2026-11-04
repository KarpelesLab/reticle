// 1024 words of 32 to infer: {'byte_we': True}.
module ib_1024x32_bytes (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [56:0] sr;
  always @(posedge clk) sr <= {sr[55:0], si};
  reg [31:0] mem [0:1023];
  wire [31:0] din = sr[41:10];
  always @(posedge clk) begin
    if (sr[52]) mem[sr[9:0]][7:0] <= din[7:0];
    if (sr[53]) mem[sr[9:0]][15:8] <= din[15:8];
    if (sr[54]) mem[sr[9:0]][23:16] <= din[23:16];
    if (sr[55]) mem[sr[9:0]][31:24] <= din[31:24];
  end
  reg [31:0] q0;
  always @(posedge clk) q0 <= mem[sr[51:42]];
  always @(posedge clk) so <= ^(q0) ^ sr[56];
endmodule
