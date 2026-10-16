// The four clock managers NewAE's ChipWhisperer-Husky gateware
// instantiates, each with its own parameters and the pins it uses:
//
// - the clock-glitch MMCM (`clockglitch_a7.v`): multiply 6, divide 6, fine
//   phase shift on CLKOUT0 driven from the fabric, and its DRP, which the
//   Husky's software reprograms;
// - the logic analyser's (`reg_la.v`): multiply 6, divide 5, no phase
//   shift, its phase-shift pins left unconnected;
// - the trace block's (`trace_top.v`): multiply 32, divide 32, fine phase
//   shift;
// - the user IO PLL (`userio.v`): a `PLLE2_ADV` multiplying by 10, with
//   both clock inputs selected by `CLKINSEL` and six dividers set. Only
//   CLKOUT0 is read here, which is what NewAE's image amounts to: its other
//   five outputs drive nothing there, and Vivado leaves them disabled.
//
// `tests/fpga_xray_cmt.rs` builds it with `reticle fpga --bitstream` and
// compares every register field of every one with what Vivado wrote for the
// same instance in NewAE's image. Nothing here has run on a part.
module clock_managers (
    input  wire clk_usb,
    input  wire sel,
    output wire LED_ADC,
    output wire LED_GLITCH,
    output wire LED_ARMED,
    output wire LED_CAP
);
  wire clk;
  BUFG u_bufg (.I(clk_usb), .O(clk));

  reg [15:0] count = 0;
  always @(posedge clk) count <= count + 1;

  // The glitch MMCM: fine phase shift and DRP from the fabric.
  wire g_out, g_fb, g_locked, g_psdone, g_drdy;
  wire [15:0] g_do;
  MMCME2_ADV #(
      .BANDWIDTH("OPTIMIZED"), .CLKFBOUT_MULT_F(6.0), .CLKOUT0_DIVIDE_F(6.0),
      .CLKFBOUT_PHASE(0.0), .CLKIN1_PERIOD(100.0), .CLKOUT0_DUTY_CYCLE(0.5),
      .CLKOUT0_PHASE(0.0), .CLKOUT4_CASCADE("FALSE"), .COMPENSATION("INTERNAL"),
      .DIVCLK_DIVIDE(1), .STARTUP_WAIT("FALSE"), .CLKFBOUT_USE_FINE_PS("FALSE"),
      .CLKOUT0_USE_FINE_PS("TRUE")
  ) u_glitch (
      .CLKOUT0(g_out), .CLKFBOUT(g_fb), .LOCKED(g_locked), .CLKIN1(clk), .CLKIN2(1'b0),
      .CLKINSEL(1'b1), .PWRDWN(count[15]), .RST(count[14]),
      .DADDR(count[6:0]), .DCLK(clk), .DEN(count[7]), .DI(count), .DWE(count[8]),
      .DO(g_do), .DRDY(g_drdy), .PSCLK(clk), .PSEN(count[9]), .PSINCDEC(count[10]),
      .PSDONE(g_psdone), .CLKFBIN(g_fb)
  );

  // The logic analyser's: phase-shift pins unconnected.
  wire l_out, l_fb, l_locked;
  MMCME2_ADV #(
      .BANDWIDTH("OPTIMIZED"), .CLKFBOUT_MULT_F(6.0), .CLKOUT0_DIVIDE_F(5.0),
      .CLKFBOUT_PHASE(0.0), .CLKIN1_PERIOD(5.0), .CLKOUT0_PHASE(0.0),
      .COMPENSATION("INTERNAL"), .DIVCLK_DIVIDE(1), .STARTUP_WAIT("FALSE"),
      .CLKFBOUT_USE_FINE_PS("FALSE"), .CLKOUT0_USE_FINE_PS("FALSE")
  ) u_la (
      .CLKOUT0(l_out), .CLKFBOUT(l_fb), .LOCKED(l_locked), .CLKIN1(clk), .CLKIN2(1'b0),
      .CLKINSEL(1'b1), .PWRDWN(count[13]), .RST(count[12]), .CLKFBIN(l_fb)
  );

  // The trace block's.
  wire t_out, t_fb, t_locked, t_psdone;
  MMCME2_ADV #(
      .BANDWIDTH("OPTIMIZED"), .CLKFBOUT_MULT_F(32.0), .CLKOUT0_DIVIDE_F(32.0),
      .CLKFBOUT_PHASE(0.0), .CLKIN1_PERIOD(100.0), .CLKOUT0_PHASE(0.0),
      .COMPENSATION("INTERNAL"), .DIVCLK_DIVIDE(1), .STARTUP_WAIT("FALSE"),
      .CLKFBOUT_USE_FINE_PS("FALSE"), .CLKOUT0_USE_FINE_PS("TRUE")
  ) u_trace (
      .CLKOUT0(t_out), .CLKFBOUT(t_fb), .LOCKED(t_locked), .CLKIN1(clk), .CLKIN2(1'b0),
      .CLKINSEL(1'b1), .PWRDWN(count[11]), .RST(count[10]), .PSCLK(clk),
      .PSEN(count[5]), .PSINCDEC(count[4]), .PSDONE(t_psdone), .CLKFBIN(t_fb)
  );

  // The user IO PLL: six outputs, two inputs, `CLKINSEL` routed.
  wire p_out;
  wire p_fb, p_locked;
  wire [15:0] p_do;
  PLLE2_ADV #(
      .BANDWIDTH("OPTIMIZED"), .CLKFBOUT_MULT(10), .CLKFBOUT_PHASE(0.0),
      .CLKIN1_PERIOD(10.4), .CLKIN2_PERIOD(5.0), .CLKOUT0_DIVIDE(10),
      .CLKOUT1_DIVIDE(20), .CLKOUT2_DIVIDE(40), .CLKOUT3_DIVIDE(60),
      .CLKOUT4_DIVIDE(120), .CLKOUT5_DIVIDE(120), .COMPENSATION("INTERNAL"),
      .DIVCLK_DIVIDE(1), .STARTUP_WAIT("FALSE")
  ) u_pll (
      .CLKOUT0(p_out), .CLKOUT1(), .CLKOUT2(), .CLKOUT3(),
      .CLKOUT4(), .CLKOUT5(), .DO(p_do), .CLKFBOUT(p_fb),
      .LOCKED(p_locked), .CLKIN1(clk), .CLKIN2(clk), .CLKINSEL(sel), .PWRDWN(count[3]),
      .RST(count[2]), .DADDR(count[6:0]), .DCLK(clk), .DEN(count[1]), .DI(count),
      .DWE(count[0]), .CLKFBIN(p_fb)
  );

  wire g_clk, l_clk, t_clk, p_clk;
  BUFG u_g (.I(g_out), .O(g_clk));
  BUFG u_l (.I(l_out), .O(l_clk));
  BUFG u_t (.I(t_out), .O(t_clk));
  BUFG u_p (.I(p_out), .O(p_clk));
  reg [3:0] g = 0, l = 0, t = 0, p = 0;
  always @(posedge g_clk) g <= g + 1;
  always @(posedge l_clk) l <= l + 1;
  always @(posedge t_clk) t <= t + 1;
  always @(posedge p_clk) p <= p + 1;
  assign LED_ADC = g[3] ^ g_locked ^ g_psdone ^ g_drdy ^ (^g_do);
  assign LED_GLITCH = l[3] ^ l_locked;
  assign LED_ARMED = t[3] ^ t_locked ^ t_psdone;
  assign LED_CAP = p[3] ^ p_locked ^ (^p_do);
endmodule
