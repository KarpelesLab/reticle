// t_s5x10
module t_s5x10 (input wire clk, input wire si, output reg so);
  reg [18:0] in;
  wire signed [4:0] a0 = in[4:0];
  wire signed [9:0] b0 = in[14:5];
  wire rr = in[17];
  wire signed [14:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[17:0], si}; so <= ^m0; end
endmodule
