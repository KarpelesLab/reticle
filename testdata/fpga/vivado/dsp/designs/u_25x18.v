// u_25x18
module u_25x18 (input wire clk, input wire si, output reg so);
  reg [46:0] in;
  wire [24:0] a0 = in[24:0];
  wire [17:0] b0 = in[42:25];
  wire rr = in[45];
  wire [42:0] m0 = a0 * b0;
  reg [42:0] m1;
  always @(posedge clk) begin in <= {in[45:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
