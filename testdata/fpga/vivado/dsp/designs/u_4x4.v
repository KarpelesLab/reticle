// u_4x4
module u_4x4 (input wire clk, input wire si, output reg so);
  reg [11:0] in;
  wire [3:0] a0 = in[3:0];
  wire [3:0] b0 = in[7:4];
  wire rr = in[10];
  wire [7:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[10:0], si}; so <= ^m0; end
endmodule
