// t_r8x8
module t_r8x8 (input wire clk, input wire si, output reg so);
  reg [19:0] in;
  wire signed [7:0] a0 = in[7:0];
  wire signed [7:0] b0 = in[15:8];
  wire rr = in[18];
  reg signed [7:0] a1;
  reg signed [7:0] b1;
  wire signed [15:0] m0 = a1 * b1;
  reg signed [15:0] m1;
  reg signed [15:0] m2;
  always @(posedge clk) begin in <= {in[18:0], si}; so <= ^m2; end
  always @(posedge clk) begin
     a1 <= a0;
     b1 <= b0;
     m1 <= m0;
     m2 <= m1;
  end
endmodule
