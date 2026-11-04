// t_s9x10
module t_s9x10 (input wire clk, input wire si, output reg so);
  reg [22:0] in;
  wire signed [8:0] a0 = in[8:0];
  wire signed [9:0] b0 = in[18:9];
  wire rr = in[21];
  wire signed [18:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[21:0], si}; so <= ^m0; end
endmodule
