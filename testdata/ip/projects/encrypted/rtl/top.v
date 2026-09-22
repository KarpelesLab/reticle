// `ddr_phy` has no source Reticle can read, so it is a black box built
// from its manifest; `vendor_pll` comes from its behavioural model.
// Both elaborate, and the design validates either way.
module top(
  input         clk,
  input         rst_n,
  output        ddr_ck,
  inout  [31:0] ddr_dq,
  output        pll_locked
);
  wire pll_clk;

  vendor_pll u_pll (
    .clk_in  (clk),
    .rst_n   (rst_n),
    .clk_out (pll_clk),
    .locked  (pll_locked)
  );

  ddr_phy u_ddr (
    .clk    (pll_clk),
    .rst_n  (rst_n),
    .ddr_ck (ddr_ck),
    .ddr_dq (ddr_dq)
  );
endmodule
