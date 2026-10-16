// Every pad configuration NewAE's ChipWhisperer-Husky gateware asks of its
// XC7A35T-FTG256, each on the ball the Husky uses it on: an LVDS_25 input
// pair through an `IBUFDS` (its PLL's reference, D13/C13), two more
// that are bits of vector ports, as the ADC's data lanes are (`ADC_DP[i]`
// and `ADC_DN[i]` from a `generate` loop, B12/A12 and A13/A14), an LVDS_25
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
    input  wire [1:0] ADC_DP,
    input  wire [1:0] ADC_DN,
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

  wire [1:0] adc_d;
  genvar i;
  generate
    for (i = 0; i < 2; i = i + 1) begin : gen_adc_data
      IBUFDS #(
          .DIFF_TERM   ("FALSE"),
          .IBUF_LOW_PWR("FALSE"),
          .IOSTANDARD  ("LVDS_25")
      ) u_adc_ibufds (
          .O (adc_d[i]),
          .I (ADC_DP[i]),
          .IB(ADC_DN[i])
      );
    end
  endgenerate

  reg [23:0] count = 0;
  always @(posedge clk) count <= count + 1;

  OBUFDS u_adc_clk (.I(count[3]), .O(ADC_CLKP), .OB(ADC_CLKN));

  PULLUP u_pull (.O(USERIO_CLK));

  assign target_io4 = count[10] ? count[11] : 1'bz;
  assign USERIO_CLK = count[12] ? count[13] : 1'bz;
  assign ADC_DFS = count[20] ^ PLL_STATUS ^ target_io4 ^ adc_d[0] ^ adc_d[1];
  assign LED_ADC = count[23] & SAM_MOSI & USERIO_CLK;
endmodule
