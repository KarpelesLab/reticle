// t_s2x18
module t_s2x18 (input wire clk, input wire si, output reg so);
  reg [23:0] in;
  wire signed [1:0] a0 = in[1:0];
  wire signed [17:0] b0 = in[19:2];
  wire rr = in[22];
  wire signed [19:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[22:0], si}; so <= ^m0; end
endmodule
