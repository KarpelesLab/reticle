// s_26x26
module s_26x26 (input wire clk, input wire si, output reg so);
  reg [55:0] in;
  wire signed [25:0] a0 = in[25:0];
  wire signed [25:0] b0 = in[51:26];
  wire rr = in[54];
  wire signed [51:0] m0 = a0 * b0;
  reg signed [51:0] m1;
  always @(posedge clk) begin in <= {in[54:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
