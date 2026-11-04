// t_r6x6
module t_r6x6 (input wire clk, input wire si, output reg so);
  reg [15:0] in;
  wire signed [5:0] a0 = in[5:0];
  wire signed [5:0] b0 = in[11:6];
  wire rr = in[14];
  reg signed [5:0] a1;
  reg signed [5:0] b1;
  wire signed [11:0] m0 = a1 * b1;
  reg signed [11:0] m1;
  reg signed [11:0] m2;
  always @(posedge clk) begin in <= {in[14:0], si}; so <= ^m2; end
  always @(posedge clk) begin
     a1 <= a0;
     b1 <= b0;
     m1 <= m0;
     m2 <= m1;
  end
endmodule
