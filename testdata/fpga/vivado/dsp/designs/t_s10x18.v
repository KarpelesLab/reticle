// t_s10x18
module t_s10x18 (input wire clk, input wire si, output reg so);
  reg [31:0] in;
  wire signed [9:0] a0 = in[9:0];
  wire signed [17:0] b0 = in[27:10];
  wire rr = in[30];
  wire signed [27:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[30:0], si}; so <= ^m0; end
endmodule
