// t_u16x16
module t_u16x16 (input wire clk, input wire si, output reg so);
  reg [35:0] in;
  wire [15:0] a0 = in[15:0];
  wire [15:0] b0 = in[31:16];
  wire rr = in[34];
  wire [31:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[34:0], si}; so <= ^m0; end
endmodule
