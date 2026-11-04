// A multiply and accumulate.
module f_macc (input wire clk, input wire si, output reg so);
  reg [35:0] in;
  reg signed [17:0] a, b;
  reg signed [47:0] acc;
  always @(posedge clk) begin
    in <= {in[34:0], si};
    a <= in[17:0];
    b <= in[35:18];
    acc <= acc + a * b;
    so <= ^acc;
  end
endmodule
