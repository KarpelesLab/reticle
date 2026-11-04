// One operand twice.
module f_square (input wire clk, input wire si, output reg so);
  reg [17:0] in;
  reg signed [17:0] a;
  reg signed [35:0] p;
  always @(posedge clk) begin
    in <= {in[16:0], si};
    a <= in;
    p <= a * a;
    so <= ^p;
  end
endmodule
