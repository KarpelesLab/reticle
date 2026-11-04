// u_8x3
module u_8x3 (input wire clk, input wire si, output reg so);
  reg [14:0] in;
  wire [7:0] a0 = in[7:0];
  wire [2:0] b0 = in[10:8];
  wire rr = in[13];
  wire [10:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[13:0], si}; so <= ^m0; end
endmodule
