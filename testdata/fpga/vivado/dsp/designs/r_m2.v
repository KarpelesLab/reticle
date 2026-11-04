// r_m2
module r_m2 (input wire clk, input wire si, output reg so);
  reg [39:0] in;
  wire signed [17:0] a0 = in[17:0];
  wire signed [17:0] b0 = in[35:18];
  wire rr = in[38];
  wire signed [35:0] m0 = a0 * b0;
  reg signed [35:0] m1;
  reg signed [35:0] m2;
  always @(posedge clk) begin in <= {in[38:0], si}; so <= ^m2; end
  always @(posedge clk) begin
     m1 <= m0;
     m2 <= m1;
  end
endmodule
