// t_s6x18
module t_s6x18 (input wire clk, input wire si, output reg so);
  reg [27:0] in;
  wire signed [5:0] a0 = in[5:0];
  wire signed [17:0] b0 = in[23:6];
  wire rr = in[26];
  wire signed [23:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[26:0], si}; so <= ^m0; end
endmodule
