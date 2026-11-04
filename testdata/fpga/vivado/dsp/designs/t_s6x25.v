// t_s6x25
module t_s6x25 (input wire clk, input wire si, output reg so);
  reg [34:0] in;
  wire signed [5:0] a0 = in[5:0];
  wire signed [24:0] b0 = in[30:6];
  wire rr = in[33];
  wire signed [30:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[33:0], si}; so <= ^m0; end
endmodule
