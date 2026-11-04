// t_u13x13
module t_u13x13 (input wire clk, input wire si, output reg so);
  reg [29:0] in;
  wire [12:0] a0 = in[12:0];
  wire [12:0] b0 = in[25:13];
  wire rr = in[28];
  wire [25:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[28:0], si}; so <= ^m0; end
endmodule
