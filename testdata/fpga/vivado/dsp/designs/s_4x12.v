// s_4x12
module s_4x12 (input wire clk, input wire si, output reg so);
  reg [19:0] in;
  wire signed [3:0] a0 = in[3:0];
  wire signed [11:0] b0 = in[15:4];
  wire rr = in[18];
  wire signed [15:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[18:0], si}; so <= ^m0; end
endmodule
