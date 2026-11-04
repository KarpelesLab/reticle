// t_u14x14
module t_u14x14 (input wire clk, input wire si, output reg so);
  reg [31:0] in;
  wire [13:0] a0 = in[13:0];
  wire [13:0] b0 = in[27:14];
  wire rr = in[30];
  wire [27:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[30:0], si}; so <= ^m0; end
endmodule
