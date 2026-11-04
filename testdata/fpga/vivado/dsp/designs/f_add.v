// A multiply and add of another value.
module f_add (input wire clk, input wire si, output reg so);
  reg [47:0] in;
  reg signed [17:0] a, b;
  reg signed [11:0] c;
  reg signed [36:0] p;
  always @(posedge clk) begin
    in <= {in[46:0], si};
    a <= in[17:0];
    b <= in[35:18];
    c <= in[47:36];
    p <= a * b + c;
    so <= ^p;
  end
endmodule
