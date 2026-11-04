// t_r11x11
module t_r11x11 (input wire clk, input wire si, output reg so);
  reg [25:0] in;
  wire signed [10:0] a0 = in[10:0];
  wire signed [10:0] b0 = in[21:11];
  wire rr = in[24];
  reg signed [10:0] a1;
  reg signed [10:0] b1;
  wire signed [21:0] m0 = a1 * b1;
  reg signed [21:0] m1;
  reg signed [21:0] m2;
  always @(posedge clk) begin in <= {in[24:0], si}; so <= ^m2; end
  always @(posedge clk) begin
     a1 <= a0;
     b1 <= b0;
     m1 <= m0;
     m2 <= m1;
  end
endmodule
