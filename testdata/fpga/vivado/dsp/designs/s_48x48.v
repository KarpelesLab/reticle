// s_48x48
module s_48x48 (input wire clk, input wire si, output reg so);
  reg [99:0] in;
  wire signed [47:0] a0 = in[47:0];
  wire signed [47:0] b0 = in[95:48];
  wire rr = in[98];
  wire signed [95:0] m0 = a0 * b0;
  reg signed [95:0] m1;
  always @(posedge clk) begin in <= {in[98:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
