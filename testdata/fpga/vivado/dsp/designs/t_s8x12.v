// t_s8x12
module t_s8x12 (input wire clk, input wire si, output reg so);
  reg [23:0] in;
  wire signed [7:0] a0 = in[7:0];
  wire signed [11:0] b0 = in[19:8];
  wire rr = in[22];
  wire signed [19:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[22:0], si}; so <= ^m0; end
endmodule
