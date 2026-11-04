// One product read twice.
module f_two (input wire clk, input wire si, output reg so);
  reg [35:0] in;
  reg signed [17:0] a, b;
  reg signed [35:0] p, q;
  wire signed [35:0] m = a * b;
  always @(posedge clk) begin
    in <= {in[34:0], si};
    a <= in[17:0];
    b <= in[35:18];
    p <= m;
    q <= m + p;
    so <= ^q;
  end
endmodule
