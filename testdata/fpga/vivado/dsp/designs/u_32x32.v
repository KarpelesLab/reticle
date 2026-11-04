// u_32x32
module u_32x32 (input wire clk, input wire si, output reg so);
  reg [67:0] in;
  wire [31:0] a0 = in[31:0];
  wire [31:0] b0 = in[63:32];
  wire rr = in[66];
  wire [63:0] m0 = a0 * b0;
  reg [63:0] m1;
  always @(posedge clk) begin in <= {in[66:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
