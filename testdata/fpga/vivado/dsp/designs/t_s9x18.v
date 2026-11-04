// t_s9x18
module t_s9x18 (input wire clk, input wire si, output reg so);
  reg [30:0] in;
  wire signed [8:0] a0 = in[8:0];
  wire signed [17:0] b0 = in[26:9];
  wire rr = in[29];
  wire signed [26:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[29:0], si}; so <= ^m0; end
endmodule
