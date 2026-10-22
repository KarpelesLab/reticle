// Every kind of global clock buffer NewAE's ChipWhisperer-Husky gateware
// instantiates by name, on the Husky's own XC7A35T-FTG256 pins: the 96 MHz
// clock on D4 through an `IBUFG` and a `BUFG`, a `BUFGMUX` choosing between
// it and a fabric-divided clock with `CLK_SEL_TYPE("ASYNC")`, as eight of
// the Husky's ten do, a second `BUFGMUX` left at its `SYNC` default, as the
// other two are, and a `BUFGCE` gating it. And the board PLL's clock on
// D13 and C13, the left side's clock-capable pair, through an `IBUFDS` and a
// `BUFG`, so that a pad on each half of the die is asked to reach its
// buffer on the dedicated road.
//
// `tests/fpga_xray_bufg.rs` builds it with `reticle fpga --bitstream` and
// compares each `BUFGCTRL`'s configuration with the three Vivado wrote into
// NewAE's own image (`src/fpga/xray/bufg.rs`). Nothing here has run on a
// part.
module clock_buffers (
    input  wire clk_usb,
    input  wire sel,
    input  wire en,
    input  wire PLLFPGAP,
    input  wire PLLFPGAN,
    output wire LED_ADC,
    output wire LED_GLITCH,
    output wire LED_ARMED,
    output wire LED_CAP
);
  wire clk_in, clk, muxed, muxed_sync, gated;
  IBUFG u_ibufg (.I(clk_usb), .O(clk_in));
  BUFG u_bufg (.I(clk_in), .O(clk));

  reg half = 1'b0;
  always @(posedge clk) half <= ~half;

  BUFGMUX #(.CLK_SEL_TYPE("ASYNC")) u_mux (.I0(clk), .I1(half), .S(sel), .O(muxed));
  BUFGMUX u_mux_sync (.I0(clk), .I1(half), .S(sel), .O(muxed_sync));
  BUFGCE u_gate (.I(clk), .CE(en), .O(gated));

  wire observer_mr, observer;
  BUFMRCE #(.INIT_OUT(0), .CE_TYPE("ASYNC")) u_mr (.I(clk), .O(observer_mr), .CE(~sel));
  BUFR #(.BUFR_DIVIDE("1")) u_r (.I(observer_mr), .O(observer), .CE(1'b1), .CLR(1'b0));
  reg [24:0] o = 0;
  always @(posedge observer) o <= o + 1;
  wire pll_in, pll;
  IBUFDS #(.IOSTANDARD("LVDS_25")) u_pll_ibuf (.I(PLLFPGAP), .IB(PLLFPGAN), .O(pll_in));
  BUFG u_pll_bufg (.I(pll_in), .O(pll));
  reg [24:0] p = 0;
  always @(posedge pll) p <= p + 1;
  assign LED_CAP = o[24] ^ p[24];

  reg [24:0] a = 0, b = 0, c = 0;
  always @(posedge muxed) a <= a + 1;
  always @(posedge muxed_sync) c <= c + 1;
  always @(posedge gated) b <= b + 1;
  assign LED_ADC = a[24];
  assign LED_GLITCH = b[24];
  assign LED_ARMED = c[24];
endmodule
