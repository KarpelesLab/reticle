// t_s15x15
module t_s15x15 (input wire clk, input wire si, output reg so);
  reg [33:0] in;
  wire signed [14:0] a0 = in[14:0];
  wire signed [14:0] b0 = in[29:15];
  wire rr = in[32];
  wire signed [29:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[32:0], si}; so <= ^m0; end
endmodule
