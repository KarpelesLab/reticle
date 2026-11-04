// t_s8x10
module t_s8x10 (input wire clk, input wire si, output reg so);
  reg [21:0] in;
  wire signed [7:0] a0 = in[7:0];
  wire signed [9:0] b0 = in[17:8];
  wire rr = in[20];
  wire signed [17:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[20:0], si}; so <= ^m0; end
endmodule
