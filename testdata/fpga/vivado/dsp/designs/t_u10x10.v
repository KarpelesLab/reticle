// t_u10x10
module t_u10x10 (input wire clk, input wire si, output reg so);
  reg [23:0] in;
  wire [9:0] a0 = in[9:0];
  wire [9:0] b0 = in[19:10];
  wire rr = in[22];
  wire [19:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[22:0], si}; so <= ^m0; end
endmodule
