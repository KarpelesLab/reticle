// u_24x17
module u_24x17 (input wire clk, input wire si, output reg so);
  reg [44:0] in;
  wire [23:0] a0 = in[23:0];
  wire [16:0] b0 = in[40:24];
  wire rr = in[43];
  wire [40:0] m0 = a0 * b0;
  reg [40:0] m1;
  always @(posedge clk) begin in <= {in[43:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
