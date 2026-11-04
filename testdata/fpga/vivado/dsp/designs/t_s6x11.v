// t_s6x11
module t_s6x11 (input wire clk, input wire si, output reg so);
  reg [20:0] in;
  wire signed [5:0] a0 = in[5:0];
  wire signed [10:0] b0 = in[16:6];
  wire rr = in[19];
  wire signed [16:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[19:0], si}; so <= ^m0; end
endmodule
