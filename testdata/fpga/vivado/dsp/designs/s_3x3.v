// s_3x3
module s_3x3 (input wire clk, input wire si, output reg so);
  reg [9:0] in;
  wire signed [2:0] a0 = in[2:0];
  wire signed [2:0] b0 = in[5:3];
  wire rr = in[8];
  wire signed [5:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[8:0], si}; so <= ^m0; end
endmodule
