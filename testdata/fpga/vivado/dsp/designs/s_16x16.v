// s_16x16
module s_16x16 (input wire clk, input wire si, output reg so);
  reg [35:0] in;
  wire signed [15:0] a0 = in[15:0];
  wire signed [15:0] b0 = in[31:16];
  wire rr = in[34];
  wire signed [31:0] m0 = a0 * b0;
  reg signed [31:0] m1;
  always @(posedge clk) begin in <= {in[34:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
