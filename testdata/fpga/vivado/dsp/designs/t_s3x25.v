// t_s3x25
module t_s3x25 (input wire clk, input wire si, output reg so);
  reg [31:0] in;
  wire signed [2:0] a0 = in[2:0];
  wire signed [24:0] b0 = in[27:3];
  wire rr = in[30];
  wire signed [27:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[30:0], si}; so <= ^m0; end
endmodule
