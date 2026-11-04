// t_u12x12
module t_u12x12 (input wire clk, input wire si, output reg so);
  reg [27:0] in;
  wire [11:0] a0 = in[11:0];
  wire [11:0] b0 = in[23:12];
  wire rr = in[26];
  wire [23:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[26:0], si}; so <= ^m0; end
endmodule
