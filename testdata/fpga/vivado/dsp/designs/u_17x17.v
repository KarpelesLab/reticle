// u_17x17
module u_17x17 (input wire clk, input wire si, output reg so);
  reg [37:0] in;
  wire [16:0] a0 = in[16:0];
  wire [16:0] b0 = in[33:17];
  wire rr = in[36];
  wire [33:0] m0 = a0 * b0;
  reg [33:0] m1;
  always @(posedge clk) begin in <= {in[36:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
