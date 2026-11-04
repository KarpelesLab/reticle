// An eight-tap filter: several multiplies and an adder chain.
module fir8 (input wire clk, input wire si, output reg so);
  reg [15:0] in;
  reg signed [15:0] t [0:7];
  reg signed [31:0] m [0:7];
  reg signed [34:0] s;
  integer i;
  always @(posedge clk) begin
    in <= {in[14:0], si};
    t[0] <= in;
    for (i = 1; i < 8; i = i + 1) t[i] <= t[i - 1];
    m[0] <= t[0] * 16'sd113;
    m[1] <= t[1] * -16'sd457;
    m[2] <= t[2] * 16'sd1291;
    m[3] <= t[3] * 16'sd5021;
    m[4] <= t[4] * 16'sd5021;
    m[5] <= t[5] * 16'sd1291;
    m[6] <= t[6] * -16'sd457;
    m[7] <= t[7] * 16'sd113;
    s <= m[0] + m[1] + m[2] + m[3] + m[4] + m[5] + m[6] + m[7];
    so <= ^s;
  end
endmodule
