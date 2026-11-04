// u_18x18
module u_18x18 (input wire clk, input wire si, output reg so);
  reg [39:0] in;
  wire [17:0] a0 = in[17:0];
  wire [17:0] b0 = in[35:18];
  wire rr = in[38];
  wire [35:0] m0 = a0 * b0;
  reg [35:0] m1;
  always @(posedge clk) begin in <= {in[38:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     m1 <= m0;
  end
endmodule
