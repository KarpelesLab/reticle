// t_s7x12
module t_s7x12 (input wire clk, input wire si, output reg so);
  reg [22:0] in;
  wire signed [6:0] a0 = in[6:0];
  wire signed [11:0] b0 = in[18:7];
  wire rr = in[21];
  wire signed [18:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[21:0], si}; so <= ^m0; end
endmodule
