// A pre-adder.
module f_preadd (input wire clk, input wire si, output reg so);
  reg [59:0] in;
  reg signed [23:0] a, d;
  reg signed [17:0] b;
  reg signed [42:0] p;
  always @(posedge clk) begin
    in <= {in[58:0], si};
    a <= in[23:0];
    d <= in[47:24];
    b <= in[59:48];
    p <= (a + d) * b;
    so <= ^p;
  end
endmodule
