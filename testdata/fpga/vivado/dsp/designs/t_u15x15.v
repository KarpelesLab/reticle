// t_u15x15
module t_u15x15 (input wire clk, input wire si, output reg so);
  reg [33:0] in;
  wire [14:0] a0 = in[14:0];
  wire [14:0] b0 = in[29:15];
  wire rr = in[32];
  wire [29:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[32:0], si}; so <= ^m0; end
endmodule
