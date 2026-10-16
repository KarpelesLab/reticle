// The registers NewAE's ChipWhisperer-Husky puts beside its pads, on the
// balls it puts them on: an `IDDR` sampling one of the ADC's LVDS data
// pairs (B12/A12) through an `IBUFDS`, and an `ODDR` forwarding a clock to
// the ADC through an `OBUFDS` (J15/J16), with the Husky's own parameters —
// `OPPOSITE_EDGE`, `SYNC`, `INIT` zero, `D1` one and `D2` zero. A third,
// single-ended `ODDR` (C7) has the Husky's glitch outputs' `ASYNC` reset
// type and a routed reset.
//
// `tests/fpga_xray_iologic.rs` builds it with `reticle fpga --bitstream`
// and compares each `ILOGIC` and `OLOGIC` with what Vivado wrote beside the
// same ball in NewAE's image. Nothing here has run on a part.
module io_registers (
    input  wire PLLFPGAP,
    input  wire PLLFPGAN,
    input  wire ADC_DP0,
    input  wire ADC_DN0,
    output wire ADC_CLKP,
    output wire ADC_CLKN,
    output wire LED_ADC,
    output wire LED_GLITCH
);
  wire ref_clk, clk;
  IBUFDS #(.DIFF_TERM("FALSE"), .IBUF_LOW_PWR("FALSE"), .IOSTANDARD("LVDS_25"))
      u_ref (.I(PLLFPGAP), .IB(PLLFPGAN), .O(ref_clk));
  BUFG u_bufg (.I(ref_clk), .O(clk));

  wire adc_bit;
  IBUFDS #(.DIFF_TERM("FALSE"), .IBUF_LOW_PWR("FALSE"), .IOSTANDARD("LVDS_25"))
      u_adc (.I(ADC_DP0), .IB(ADC_DN0), .O(adc_bit));
  wire rise, fall;
  IDDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE"), .INIT_Q1(0), .INIT_Q2(0), .SRTYPE("SYNC"))
      u_iddr (.Q1(rise), .Q2(fall), .D(adc_bit), .CE(1'b1), .C(clk), .S(1'b0), .R(1'b0));

  wire adc_clk;
  ODDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE"), .INIT(1'b0), .SRTYPE("SYNC"))
      u_oddr (.Q(adc_clk), .C(clk), .CE(1'b1), .D1(1'b1), .D2(1'b0), .R(1'b0), .S(1'b0));
  OBUFDS u_adc_clk (.I(adc_clk), .O(ADC_CLKP), .OB(ADC_CLKN));

  reg [22:0] count = 0;
  always @(posedge clk) count <= count + 1;
  wire glitch;
  ODDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE"), .INIT(1'b0), .SRTYPE("ASYNC"))
      u_glitch (.Q(glitch), .C(clk), .CE(count[2]), .D1(1'b1), .D2(1'b0), .R(~count[2]), .S(1'b0));
  assign LED_GLITCH = glitch;
  assign LED_ADC = count[22] ^ rise ^ fall;
endmodule
