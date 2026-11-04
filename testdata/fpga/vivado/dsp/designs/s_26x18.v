// s_26x18
module s_26x18 (input wire clk, input wire si, output reg so);
  reg [47:0] in;
  wire signed [25:0] a0 = in[25:0];
  wire signed [17:0] b0 = in[43:26];
  wire rr = in[46];
  wire signed [43:0] m0 = a0 * b0;
  reg signed [43:0] m1;
  always @(posedge clk) begin in <= {in[46:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
