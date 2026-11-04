// t_s9x9
module t_s9x9 (input wire clk, input wire si, output reg so);
  reg [21:0] in;
  wire signed [8:0] a0 = in[8:0];
  wire signed [8:0] b0 = in[17:9];
  wire rr = in[20];
  wire signed [17:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[20:0], si}; so <= ^m0; end
endmodule
