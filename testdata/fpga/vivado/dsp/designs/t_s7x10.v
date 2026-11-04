// t_s7x10
module t_s7x10 (input wire clk, input wire si, output reg so);
  reg [20:0] in;
  wire signed [6:0] a0 = in[6:0];
  wire signed [9:0] b0 = in[16:7];
  wire rr = in[19];
  wire signed [16:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[19:0], si}; so <= ^m0; end
endmodule
