// s_35x35
module s_35x35 (input wire clk, input wire si, output reg so);
  reg [73:0] in;
  wire signed [34:0] a0 = in[34:0];
  wire signed [34:0] b0 = in[69:35];
  wire rr = in[72];
  wire signed [69:0] m0 = a0 * b0;
  reg signed [69:0] m1;
  always @(posedge clk) begin in <= {in[72:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
