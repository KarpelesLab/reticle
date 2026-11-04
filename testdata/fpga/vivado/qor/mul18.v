// A registered 18-by-18 multiply: what a DSP48E1 is for.
module mul18 (input wire clk, input wire si, output reg so);
  reg [35:0] in;
  reg signed [17:0] a, b;
  reg signed [35:0] p, q;
  always @(posedge clk) begin
    in <= {in[34:0], si};
    a <= in[17:0];
    b <= in[35:18];
    p <= a * b;
    q <= p;
    so <= ^q;
  end
endmodule
