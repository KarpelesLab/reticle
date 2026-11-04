// t_s5x18
module t_s5x18 (input wire clk, input wire si, output reg so);
  reg [26:0] in;
  wire signed [4:0] a0 = in[4:0];
  wire signed [17:0] b0 = in[22:5];
  wire rr = in[25];
  wire signed [22:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[25:0], si}; so <= ^m0; end
endmodule
