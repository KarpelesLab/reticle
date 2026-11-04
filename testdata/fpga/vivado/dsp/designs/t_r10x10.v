// t_r10x10
module t_r10x10 (input wire clk, input wire si, output reg so);
  reg [23:0] in;
  wire signed [9:0] a0 = in[9:0];
  wire signed [9:0] b0 = in[19:10];
  wire rr = in[22];
  reg signed [9:0] a1;
  reg signed [9:0] b1;
  wire signed [19:0] m0 = a1 * b1;
  reg signed [19:0] m1;
  reg signed [19:0] m2;
  always @(posedge clk) begin in <= {in[22:0], si}; so <= ^m2; end
  always @(posedge clk) begin
     a1 <= a0;
     b1 <= b0;
     m1 <= m0;
     m2 <= m1;
  end
endmodule
