// By a power of two, which is a shift.
module k_pow2 (input wire clk, input wire si, output reg so);
  reg [17:0] in;
  reg signed [35:0] m;
  always @(posedge clk) begin
    in <= {in[16:0], si};
    m <= $signed(in) * 18'sd1024;
    so <= ^m;
  end
endmodule
