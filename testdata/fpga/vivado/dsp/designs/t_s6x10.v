// t_s6x10
module t_s6x10 (input wire clk, input wire si, output reg so);
  reg [19:0] in;
  wire signed [5:0] a0 = in[5:0];
  wire signed [9:0] b0 = in[15:6];
  wire rr = in[18];
  wire signed [15:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[18:0], si}; so <= ^m0; end
endmodule
