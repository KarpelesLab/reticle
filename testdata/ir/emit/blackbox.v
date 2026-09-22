module pll_wrap (
  input wire clk_in,
  output wire clk_out,
  output wire locked,
  output wire led
);
  (* keep = 1 *)
  SB_PLL40_CORE #(.DIVF(7'h3f), .FEEDBACK_PATH("SIMPLE")) pll (
    .REFERENCECLK(clk_in),
    .PLLOUTCORE(clk_out),
    .LOCK(locked)
  );
  vendor_blinky blink (
    .clk(clk_out),
    .led(led)
  );
endmodule
