// u_6x6
module u_6x6 (input wire clk, input wire si, output reg so);
  reg [15:0] in;
  wire [5:0] a0 = in[5:0];
  wire [5:0] b0 = in[11:6];
  wire rr = in[14];
  wire [11:0] m0 = a0 * b0;
  always @(posedge clk) begin in <= {in[14:0], si}; so <= ^m0; end
endmodule
