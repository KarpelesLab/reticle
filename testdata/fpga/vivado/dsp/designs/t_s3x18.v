// t_s3x18
module t_s3x18 (input wire clk, input wire si, output reg so);
  reg [24:0] in;
  wire signed [2:0] a0 = in[2:0];
  wire signed [17:0] b0 = in[20:3];
  wire rr = in[23];
  wire signed [20:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[23:0], si}; so <= ^m0; end
endmodule
