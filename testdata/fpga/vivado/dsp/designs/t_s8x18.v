// t_s8x18
module t_s8x18 (input wire clk, input wire si, output reg so);
  reg [29:0] in;
  wire signed [7:0] a0 = in[7:0];
  wire signed [17:0] b0 = in[25:8];
  wire rr = in[28];
  wire signed [25:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[28:0], si}; so <= ^m0; end
endmodule
