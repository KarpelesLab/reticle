// A counter off a Cynthion's own 60 MHz oscillator, blinking two of its
// LEDs in antiphase.
//
// ===================================================================
// WHAT A PERSON SHOULD WATCH, AND AT WHAT RATE
// ===================================================================
//
// **Nothing has to be pressed.** No button, no switch, no host software.
// The board's oscillator runs as soon as it is powered, so this starts the
// moment the bitstream is loaded and never stops.
//
// **Which LEDs.** The six FPGA LEDs are the row of six beside the
// silkscreen legend `FPGA LEDs`. They are **not** individually numbered on
// the board — the r1.4.0 silkscreen has that one legend and no digits — so
// the two to watch are identified by geometry instead:
//
//     the two at the end of the row FARTHEST from the USER button.
//
// That is `led_n[0]` (the very end) and `led_n[1]` (next to it). On the
// r1.4.0 layout `led_n[0]` is the diode `D7` and `led_n[5]` is `D2`, and
// `D7` is the end of the row away from the button; the five LEDs lettered
// A to E elsewhere on the board belong to the debug microcontroller and
// this design does not touch them.
//
// **What they do.** The two swap, over and over, and **one of them is lit
// at every instant**:
//
//     far end of the row:   LIT   dark  LIT   dark  LIT   dark ...
//     next to it:           dark  LIT   dark  LIT   dark  LIT  ...
//     the other four:       dark  dark  dark  dark  dark  dark
//
// **At what rate.** Each LED is on for **0.56 s** and off for 0.56 s, so
// it blinks at **0.89 Hz** — a little slower than one blink a second, slow
// enough to count against a watch and far too slow to be mistaken for a
// flicker. Ten full blinks take **11.2 seconds**.
//
// The arithmetic, so a wrong counter bit is a wrong rate rather than a
// shrug: the oscillator is 60.000 MHz, bit 25 of the counter changes every
// 2^25 cycles, and 2^25 / 60e6 = 0.5592 s. One full period is twice that,
// 1.118 s, hence 0.894 Hz. A counter bit out by one would read 0.45 Hz or
// 1.8 Hz, which is the difference between "about one a second" and
// "obviously not".
//
// ===================================================================
// WHY THE OBSERVATION IS SPELLED THIS WAY
// ===================================================================
//
// Two LEDs in antiphase rather than one blinking, for the same reason
// `button_led.v` beat `leds.v`: **exactly one of the two lit, always, is a
// steady state nothing else on this board produces.** A dark board, a
// board still running the analyzer gateware from flash, a bitstream whose
// bank rail is missing, and a configuration that did nothing all look
// alike; two LEDs trading places at about one hertz does not look like any
// of them. It also says the clock is running rather than merely present:
// if the clock were stuck the pair would freeze in one of its two states,
// which is visibly different from both of them being dark.
//
// LEDs 2 to 5 are driven high — dark — rather than left alone, so what a
// person sees is "two of six moving and four dark" and not "two of six
// moving and four unknown".
//
// ===================================================================
// WHAT IT EXERCISES
// ===================================================================
//
// This is the first ECP5 design of this project with a clock. It needs,
// beyond everything `button_led.v` needed:
//
//   * **a flip-flop**, which on this family places on its own — `M<n>_SLICE`
//     is a mux output the interconnect drives, so no lookup table has to
//     be packed with it — and whose `CEMUX` default is the one setting
//     here that would leave a whole design frozen if it were missed: the
//     field defaults to "take the enable from the fabric", and nothing
//     drives that wire;
//   * **the global clock network**, which is the whole of what was left of
//     this fabric: `globals.json`'s quadrants, tap columns and spines, and
//     the three joins between them that `bits.db` states nowhere because
//     the network's wires carry the same name in every tile they cross;
//   * **a clock arriving on a pad that is not a dedicated clock pad.**
//     Ball A8 is `(row 0, col 29, PIO B)` in `iodb.json`, whose pin
//     function is `PCLKC0_0` — the *complement* half of bank 0's
//     differential clock pair. Only the `PCLKT` half has the dedicated
//     path to the centre mux (`G_JPCLKT01 <- JINCK <- JPADDI`), so this
//     clock reaches its buffer the way nextpnr would route it: through
//     ordinary interconnect into one of the `PCLKCIB` wires that feed a
//     `DCC`. That is not a shortcut, it is what the board's wiring forces;
//   * and **a mux whose sources want bits clear.** Every centre mux of
//     this die encodes its source as a six-bit code, so selecting one
//     means leaving five bits alone that another feature could set. See
//     `docs/fpga-trellis.md` on what is done about that.
//
// Pins: testdata/fpga/cynthion/clock_blink.rcf.

module clock_blink (
    input  wire       clk,
    output wire [5:0] led_n
);

    reg [25:0] count = 26'd0;

    // `toggle[i]` is the carry into bit i: one when every lower bit is
    // one, so `count ^ toggle` is `count + 1` exactly. Written as a
    // reduction rather than as a carry chain because `CCU2C` has no port
    // map in `src/fpga/devices/ecp5.dev` — its two sum bits and internal
    // carry do not match the `(ci, i0, i1) -> co` model Reticle maps carry
    // onto — and as a reduction rather than a chain of ANDs so that
    // mapping gets a balanced tree and not a 25-deep ripple.
    //
    // `examples/basys3/blink.v` is the same module for a Xilinx part, and
    // `tests/fpga_xray.rs` proves that spelling equivalent to `count + 1`
    // with Reticle's own equivalence checker. Nothing about the argument
    // is vendor-specific.
    wire [25:0] toggle;
    assign toggle[0] = 1'b1;

    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : carry
            assign toggle[i] = &count[i-1:0];
        end
    endgenerate

    always @(posedge clk) begin
        count <= count ^ toggle;
    end

    // Active low: a zero lights a LED. So the far end of the row is lit
    // while bit 25 is one, the next one along while it is zero, and the two
    // are never in the same state.
    assign led_n[0] = ~count[25];
    assign led_n[1] =  count[25];

    // Dark, always. A one is dark; see the header.
    assign led_n[5:2] = 4'b1111;

endmodule
