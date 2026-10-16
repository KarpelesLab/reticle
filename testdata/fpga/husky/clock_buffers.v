// Every kind of global clock buffer NewAE's ChipWhisperer-Husky gateware
// instantiates by name, on the Husky's own XC7A35T-FTG256 pins: the 96 MHz
// clock on D4 through an `IBUFG` and a `BUFG`, a `BUFGMUX` choosing between
// it and a fabric-divided clock with `CLK_SEL_TYPE("ASYNC")`, as eight of
// the Husky's ten do, a second `BUFGMUX` left at its `SYNC` default, as the
// other two are, and a `BUFGCE` gating it.
//
// `tests/fpga_xray_bufg.rs` builds it with `reticle fpga --bitstream` and
// compares each `BUFGCTRL`'s configuration with the three Vivado wrote into
// NewAE's own image (`src/fpga/xray/bufg.rs`). Nothing here has run on a
// part.
module clock_buffers (
    input  wire clk_usb,
    input  wire sel,
    input  wire en,
    output wire LED_ADC,
    output wire LED_GLITCH,
    output wire LED_ARMED
);
  wire clk_in, clk, muxed, muxed_sync, gated;
  IBUFG u_ibufg (.I(clk_usb), .O(clk_in));
  BUFG u_bufg (.I(clk_in), .O(clk));

  reg half = 1'b0;
  always @(posedge clk) half <= ~half;

  BUFGMUX #(.CLK_SEL_TYPE("ASYNC")) u_mux (.I0(clk), .I1(half), .S(sel), .O(muxed));
  BUFGMUX u_mux_sync (.I0(clk), .I1(half), .S(sel), .O(muxed_sync));
  BUFGCE u_gate (.I(clk), .CE(en), .O(gated));

  reg [24:0] a = 0, b = 0, c = 0;
  always @(posedge muxed) a <= a + 1;
  always @(posedge muxed_sync) c <= c + 1;
  always @(posedge gated) b <= b + 1;
  assign LED_ADC = a[24];
  assign LED_GLITCH = b[24];
  assign LED_ARMED = c[24];
endmodule
