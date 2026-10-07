// Two counters on a Digilent Basys 3, one off the board's oscillator and
// one off a PLL, blinking two LEDs that should keep exact step with each
// other — and the PLL's LOCKED on a third.
//
// WHAT A PERSON SHOULD SEE:
//
// - LED 0 (U16, the right-hand end of the row) blinks at 1.49 Hz, about
//   three times every two seconds. It counts the raw 100 MHz oscillator,
//   exactly as `blink.v` does.
// - LED 1 (E19, the one to its left) blinks at **the same rate, in step
//   with LED 0**. It counts the PLL's output, 25 MHz, with a counter two
//   bits shorter: 25 MHz / 2^24 is 100 MHz / 2^26.
// - LED 2 (U19) is lit steadily. It is the PLL's LOCKED.
// - Every other LED stays dark.
//
// Why "in step" is the test: the PLL multiplies by 8 and divides by 32,
// and every one of those numbers lives in the bitstream as a register the
// flow computed. A wrong divider shows as LED 1 running at another rate:
// twice or half as fast, or, for a divider one count off (31 or 33),
// about 3 % fast or slow, so that the two LEDs are blinking in opposition
// within ten to fifteen seconds. Two LEDs still in step after a minute
// mean the PLL's output is within about half a percent of 25 MHz. A
// single LED blinking would only say that some clock arrives.
//
// LED 2 dark with LED 1 blinking anyway would mean the PLL is running
// unlocked. LED 1 dark would mean no clock comes out of the PLL at all.
// LED 0 dark would mean the build is broken in a way `blink.v` already
// rules out.
//
// THE PLL: VCO = 100 MHz * CLKFBOUT_MULT 8 / DIVCLK_DIVIDE 1 = 800 MHz,
// the bottom of the -1 speed grade's 800-1600 MHz band, and CLKOUT0 =
// 800 / 32 = 25 MHz. Its reference is the oscillator after the raw
// clock's own BUFG, along the clock row to the PLL's input mux, and
// CLKFBOUT goes straight back to CLKFBIN inside the clock management
// tile (`docs/fpga-xray.md`, "The PLL"). Its reset and power-down are
// tied low.
//
// WHAT HAS BEEN TRIED ON A PART: NOTHING. Built by this flow, every set
// bit decodes back to a named feature; nobody has loaded it yet.
//
// The increments are toggle chains, as in `blink.v`, because a 7-series
// carry chain does not route here yet; `blink.v` says why and
// `tests/fpga_xray.rs` proves that form equal to `count + 1`.
module pll_blink (
    input  wire clk,
    output wire led_raw,
    output wire led_pll,
    output wire led_locked
);
    // ---- The PLL's output: asked for by frequency, not instantiated. ----
    //
    // A clock net nothing drives, with a frequency on it, is how a design
    // asks Reticle for a PLL (`docs/fpga.md`, "Generated clocks"). The
    // solver picks DIVCLK_DIVIDE 1, CLKFBOUT_MULT 8, CLKOUT0_DIVIDE 32:
    // the first exact answer, with the VCO at 800 MHz.
    (* clock_mhz = 25 *) wire clk_pll;
    // And a net nothing drives that names that clock is its LOCKED.
    (* clock_locked = "clk_pll" *) wire locked;

    // ---- 26 bits at 100 MHz: bit 25 toggles every 2^25 cycles. ----
    reg [25:0] raw_count = 26'd0;
    wire [25:0] raw_toggle;
    assign raw_toggle[0] = 1'b1;
    // ---- 24 bits at 25 MHz: bit 23 toggles every 2^23 cycles. ----
    reg [23:0] pll_count = 24'd0;
    wire [23:0] pll_toggle;
    assign pll_toggle[0] = 1'b1;

    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : raw_carry
            assign raw_toggle[i] = &raw_count[i-1:0];
        end
        for (i = 1; i < 24; i = i + 1) begin : pll_carry
            assign pll_toggle[i] = &pll_count[i-1:0];
        end
    endgenerate

    always @(posedge clk) raw_count <= raw_count ^ raw_toggle;
    always @(posedge clk_pll) pll_count <= pll_count ^ pll_toggle;

    assign led_raw = raw_count[25];
    assign led_pll = pll_count[23];
    assign led_locked = locked;
endmodule
