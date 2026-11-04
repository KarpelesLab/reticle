// t_s4x18
module t_s4x18 (input wire clk, input wire si, output reg so);
  reg [25:0] in;
  wire signed [3:0] a0 = in[3:0];
  wire signed [17:0] b0 = in[21:4];
  wire rr = in[24];
  wire signed [21:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[24:0], si}; so <= ^m0; end
endmodule
