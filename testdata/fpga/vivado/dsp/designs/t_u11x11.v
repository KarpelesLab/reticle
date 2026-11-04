// t_u11x11
module t_u11x11 (input wire clk, input wire si, output reg so);
  reg [25:0] in;
  wire [10:0] a0 = in[10:0];
  wire [10:0] b0 = in[21:11];
  wire rr = in[24];
  wire [21:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[24:0], si}; so <= ^m0; end
endmodule
