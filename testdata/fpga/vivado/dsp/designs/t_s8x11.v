// t_s8x11
module t_s8x11 (input wire clk, input wire si, output reg so);
  reg [22:0] in;
  wire signed [7:0] a0 = in[7:0];
  wire signed [10:0] b0 = in[18:8];
  wire rr = in[21];
  wire signed [18:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[21:0], si}; so <= ^m0; end
endmodule
