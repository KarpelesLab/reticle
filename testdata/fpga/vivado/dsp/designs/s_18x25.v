// s_18x25
module s_18x25 (input wire clk, input wire si, output reg so);
  reg [46:0] in;
  wire signed [17:0] a0 = in[17:0];
  wire signed [24:0] b0 = in[42:18];
  wire rr = in[45];
  wire signed [42:0] m0 = a0 * b0;
  reg signed [42:0] m1;
  always @(posedge clk) begin in <= {in[45:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
