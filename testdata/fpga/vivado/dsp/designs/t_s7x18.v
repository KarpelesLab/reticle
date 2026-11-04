// t_s7x18
module t_s7x18 (input wire clk, input wire si, output reg so);
  reg [28:0] in;
  wire signed [6:0] a0 = in[6:0];
  wire signed [17:0] b0 = in[24:7];
  wire rr = in[27];
  wire signed [24:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[27:0], si}; so <= ^m0; end
endmodule
