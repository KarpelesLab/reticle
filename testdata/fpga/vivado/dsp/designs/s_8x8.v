// s_8x8
module s_8x8 (input wire clk, input wire si, output reg so);
  reg [19:0] in;
  wire signed [7:0] a0 = in[7:0];
  wire signed [7:0] b0 = in[15:8];
  wire rr = in[18];
  wire signed [15:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[18:0], si}; so <= ^m0; end
endmodule
