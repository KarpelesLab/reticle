// Every pad configuration NewAE's ChipWhisperer-Husky gateware asks of its
// XC7A35T-FTG256, each on the ball the Husky uses it on: an LVDS_25 input
// pair through an `IBUFDS` (its PLL's reference, D13/C13), an LVDS_25
// output pair through an `OBUFDS` (the ADC's clock, J15/J16), an LVCMOS25
// output and input (G12, C11), an LVCMOS33 output at 8 mA (an LED, C7), an
// LVCMOS33 input (T7), and two bidirectional pads, one pulled down (T4) by
// its constraint and one pulled up (N12) by a `PULLUP` cell, which is how
// `userio.v` pulls it.
//
// `tests/fpga_xray_io.rs` builds it with `reticle fpga --bitstream` and
// compares each pad, half by half, with what Vivado wrote on the same ball
// of NewAE's own image. Nothing here has run on a part.
module io_standards (
    input  wire PLLFPGAP,
    input  wire PLLFPGAN,
    output wire ADC_CLKP,
    output wire ADC_CLKN,
    output wire ADC_DFS,
    input  wire PLL_STATUS,
    output wire LED_ADC,
    input  wire SAM_MOSI,
    inout  wire target_io4,
    inout  wire USERIO_CLK
);
  wire ref_clk, clk;
  IBUFDS #(
      .DIFF_TERM   ("FALSE"),
      .IBUF_LOW_PWR("FALSE"),
      .IOSTANDARD  ("LVDS_25")
  ) u_ref (
      .I (PLLFPGAP),
      .IB(PLLFPGAN),
      .O (ref_clk)
  );
  BUFG u_bufg (.I(ref_clk), .O(clk));

  reg [23:0] count = 0;
  always @(posedge clk) count <= count + 1;

  OBUFDS u_adc_clk (.I(count[3]), .O(ADC_CLKP), .OB(ADC_CLKN));

  PULLUP u_pull (.O(USERIO_CLK));

  assign target_io4 = count[10] ? count[11] : 1'bz;
  assign USERIO_CLK = count[12] ? count[13] : 1'bz;
  assign ADC_DFS = count[20] ^ PLL_STATUS ^ target_io4;
  assign LED_ADC = count[23] & SAM_MOSI & USERIO_CLK;
endmodule
