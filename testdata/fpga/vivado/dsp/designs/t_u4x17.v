// t_u4x17
module t_u4x17 (input wire clk, input wire si, output reg so);
  reg [24:0] in;
  wire [3:0] a0 = in[3:0];
  wire [16:0] b0 = in[20:4];
  wire rr = in[23];
  wire [20:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[23:0], si}; so <= ^m0; end
endmodule
