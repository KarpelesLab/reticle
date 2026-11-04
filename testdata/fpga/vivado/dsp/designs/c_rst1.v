// c_rst1
module c_rst1 (input wire clk, input wire si, output reg so);
  reg [39:0] in;
  wire signed [17:0] a0 = in[17:0];
  wire signed [17:0] b0 = in[35:18];
  wire rr = in[38];
  reg signed [17:0] a1;
  reg signed [17:0] b1;
  wire signed [35:0] m0 = a1 * b1;
  reg signed [35:0] m1;
  reg signed [35:0] m2;
  always @(posedge clk) begin in <= {in[38:0], si}; so <= ^m2; end
  always @(posedge clk) begin
    if (rr) a1 <= 1; else  a1 <= a0;
    if (rr) b1 <= 1; else  b1 <= b0;
    if (rr) m1 <= 1; else  m1 <= m0;
    if (rr) m2 <= 1; else  m2 <= m1;
  end
endmodule
