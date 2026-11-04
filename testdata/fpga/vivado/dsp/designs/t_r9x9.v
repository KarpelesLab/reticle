// t_r9x9
module t_r9x9 (input wire clk, input wire si, output reg so);
  reg [21:0] in;
  wire signed [8:0] a0 = in[8:0];
  wire signed [8:0] b0 = in[17:9];
  wire rr = in[20];
  reg signed [8:0] a1;
  reg signed [8:0] b1;
  wire signed [17:0] m0 = a1 * b1;
  reg signed [17:0] m1;
  reg signed [17:0] m2;
  always @(posedge clk) begin in <= {in[20:0], si}; so <= ^m2; end
  always @(posedge clk) begin
     a1 <= a0;
     b1 <= b0;
     m1 <= m0;
     m2 <= m1;
  end
endmodule
