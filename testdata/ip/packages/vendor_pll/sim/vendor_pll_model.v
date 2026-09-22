// The behavioural model: not the real PLL, but enough of it to
// simulate. `elaborate_project` uses this in place of the encrypted
// source and says so in its report.
module vendor_pll(
  input  clk_in,
  input  rst_n,
  output clk_out,
  output locked
);
  reg locked_q;

  assign clk_out = clk_in;
  assign locked  = locked_q;

  always @(posedge clk_in or negedge rst_n) begin
    if (!rst_n) locked_q <= 1'b0;
    else        locked_q <= 1'b1;
  end
endmodule
