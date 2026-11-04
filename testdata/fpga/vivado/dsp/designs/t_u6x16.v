// t_u6x16
module t_u6x16 (input wire clk, input wire si, output reg so);
  reg [25:0] in;
  wire [5:0] a0 = in[5:0];
  wire [15:0] b0 = in[21:6];
  wire rr = in[24];
  wire [21:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[24:0], si}; so <= ^m0; end
endmodule
