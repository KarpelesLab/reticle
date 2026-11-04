// s_25x19
module s_25x19 (input wire clk, input wire si, output reg so);
  reg [47:0] in;
  wire signed [24:0] a0 = in[24:0];
  wire signed [18:0] b0 = in[43:25];
  wire rr = in[46];
  wire signed [43:0] m0 = a0 * b0;
  reg signed [43:0] m1;
  always @(posedge clk) begin in <= {in[46:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
