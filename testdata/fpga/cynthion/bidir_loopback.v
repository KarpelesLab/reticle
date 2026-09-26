// A Cynthion pad that drives, releases, and reads itself back: one pin
// turned around by the USER button, with the turnaround made visible.
//
// ===================================================================
// WHAT A PERSON SHOULD PRESS, AND WHAT THEY SHOULD SEE
// ===================================================================
//
// **Press and hold the button silkscreened `USER`.** The board has three
// tactile buttons and the other two end the experiment rather than perform
// it: `PROG` (or `PROGRAM`) makes the FPGA reload itself from flash, and
// `RESET` resets the debug microcontroller. On the r1.4.0 layout `USER`
// and `PROG` are on the same edge of the board and `RESET` is on the
// opposite one (`SW3`, `SW1` and `SW2` of `cynthion.kicad_pcb`).
//
// **Which LEDs to watch.** All six, this time. The six FPGA LEDs are the
// row of six beside the silkscreen legend `FPGA LEDs`. They are **not**
// individually numbered on the board — the r1.4.0 silkscreen has that one
// legend and no digits — so they are counted from the end of the row
// FARTHEST from the USER button, which is `led_n[0]` and the diode `D7`:
//
//     LED 0  the far end of the row      <- THE BIDIRECTIONAL PAD
//     LED 1  next to it
//     LED 2  next to that
//     LED 3  the fourth
//     LED 4, LED 5  the two nearest the button
//
// With this design in the part and **nobody touching the board**:
//
//     LED 0   dark
//     LED 1   dark
//     LED 2   LIT, and it does not move
//     LED 3   blinking, about once a second
//     LED 4   dark
//     LED 5   dark
//
// While the `USER` button is **held down**:
//
//     LED 0   blinking
//     LED 1   blinking, lit at the same instants as LED 0
//     LED 2   blinking, lit exactly when LED 0 and LED 1 are dark
//     LED 3   blinking, lit at the same instants as LED 0
//     LED 4   dark
//     LED 5   dark
//
// and it goes back when the button is let go. So: **one LED blinks on its
// own until the button is held, and then four of the six are moving.**
// LED 2 is the one to look at first, because it is the one that is lit and
// *still* with nothing pressed and starts moving the moment a finger
// arrives.
//
// At any one instant, what is lit is:
//
//     nothing pressed:   LED 2 and LED 3     or  LED 2 alone
//     `USER` held:       LED 0, 1 and 3      or  LED 2 alone
//
// alternating every 0.56 s. LED 1 and LED 2 are complements, so exactly one
// of the two is lit at every instant either way.
//
// **At what rate.** LED 3 is on for 0.56 s and off for 0.56 s — 0.89 Hz,
// a little slower than one blink a second. Ten full blinks take 11.2
// seconds. The oscillator is 60.000 MHz, bit 25 of the counter changes
// every 2^25 cycles, and 2^25 / 60e6 = 0.5592 s; a counter bit out by one
// would read 0.45 Hz or 1.8 Hz.
//
// **What would be wrong, and what it would look like.** This is why the
// observation is spelled with four LEDs rather than one:
//
//   * **the output enable inverted** — LED 0, 1 and 2 move with nothing
//     pressed and freeze when the button is held;
//   * **the pad never released** (the tristate tied instead of routed) —
//     LED 0 and LED 1 blink whether or not anybody presses anything;
//   * **the pad never driven** — LED 0 never lights and LED 2 never moves;
//   * **the input path dead** — LED 1 and LED 2 never move, whatever LED 0
//     does;
//   * **the pull-up missing** (`PULLMODE` left at the database's default,
//     which is a pull-*down*) — with nothing pressed LED 1 is lit and
//     LED 2 is dark, the other way round from the table, or the two of
//     them flicker;
//   * **the clock stopped** — LED 3 is frozen.
//
// Every one of those is a different picture, and none of them is the one
// above.
//
// ===================================================================
// WHY THIS PIN, AND WHY IT IS SAFE TO DRIVE
// ===================================================================
//
// The bidirectional pad is ball **E13**, which is `led_n[0]`, the LED at
// the far end of the row. **Driving a ball that something else on the
// board also drives can damage hardware**, so the reason this one is free
// is worth writing out:
//
// 1. **Great Scott Gadgets' own platform file mentions E13 exactly once**,
//    in `cynthion/python/src/gateware/platform/cynthion_r1_4.py`:
//
//        *LEDResources(pins="E13 C13 B14 A15 D12 C11",
//                      attrs=Attrs(IO_TYPE="LVCMOS33"), invert=True),
//
//    Nothing else — no ULPI, no HyperRAM, no Type-C controller, no
//    pseudo-supply pin, no PMOD or mezzanine connector pin — claims it.
// 2. **The schematic says what is on the net and it is passive.** In
//    `indicators_buttons.kicad_sch` all six LED anodes sit on one wire up
//    to the `+3V3` symbol and each cathode goes through a series resistor
//    to the FPGA. So the only things on E13 are a resistor and a diode to
//    a supply rail. **The FPGA is the only driver**, which is the whole
//    question.
// 3. **This project has already driven it low**, in `leds.v`,
//    `button_led.v` and `clock_blink.v`, and a person watched each of the
//    three. Releasing a pin is strictly less demanding than driving it.
// 4. **A released E13 settles high, and nothing is stressed.** The only
//    current path is +3V3 through the resistor and the LED *into* the
//    pad, so the board can only pull the pin up, never down; with the
//    internal pull-up pulling the same way the pad sits at VCCIO, no
//    current flows through the LED, and the readback is a firm one. That
//    is also why the LED is dark when the pad is released rather than
//    dimly lit.
//
// The textbook answer would have been a PMOD pin — PMOD A is C9 B9 D11
// C12 C8 D8 D9 C10 and PMOD B is B4 B5 B6 B7 C5 A5 A6 A7, all of them
// `dir="io"` user IO on the top edge — and it was **not** taken, for one
// reason: those go to a 2x6 header, and nothing this machine can read says
// whether anything is plugged into it. A pin whose net is fully described
// by a schematic and has no other driver is a better bet than a pin that
// is probably unconnected. The only ball on the two edges this backend
// describes that the platform file never mentions at all is **B3**, and it
// is a worse choice for the same reason turned around: the file's silence
// is not a statement that the ball is free.
//
// ===================================================================
// WHY THE LEDS ARE WIRED UP THE WAY THEY ARE
// ===================================================================
//
// Everything on this board is active low and the inversions cancel in
// places, so each assignment is worth reading once.
//
// The LEDs have their anodes on +3V3 and their cathodes on the FPGA, so a
// pin driven **low** lights one. The button has a 10 k pull-up to +3V3 and
// shorts to ground when pressed, so `button_n` reads **low** while it is
// held. Hence:
//
//   * the pad's output enable is `~button_n`, so it **drives while the
//     button is held** and is released the rest of the time;
//   * LED 0 *is* the pad, so it is lit exactly while the pad is being
//     driven low, which is while the button is held and the counter bit is
//     zero;
//   * `read_low_n = probe_n` lights LED 1 when the pin **reads low**, and
//     `read_high_n = ~probe_n` lights LED 2 when it **reads high**. The
//     two are complements, so exactly one of them is lit at every instant
//     — a state nothing else on this board produces;
//   * `heartbeat_n = blink` lights LED 3 when the counter bit is zero,
//     which is the same instant the pad is driven low. So LED 3 blinking
//     alone says the clock runs and the pad is released, and LED 0 and
//     LED 1 joining it **in step** says what went out came back in.
//
// The lockstep is the point. LED 0 shows the pin being pulled low from
// inside the part; LED 1 shows the *input buffer of the same pin* seeing
// it; LED 3 shows what the design meant. Three LEDs agreeing, and a
// fourth in opposition, is a turnaround seen rather than inferred.
//
// ===================================================================
// WHAT IT EXERCISES
// ===================================================================
//
// This is the first design of this project, on any family, with a
// genuinely bidirectional pad. Beyond everything `clock_blink.v` needed it
// needs:
//
//   * **a tri-state driver in the source that becomes a pad's enable.**
//     `assign probe_n = ~button_n ? blink : 1'bz;` lowers to the IR's
//     `tristate` cell, and `fpga::primitives`' IO pass takes that cell
//     over: its data becomes the buffer's `I`, its enable becomes the
//     buffer's `T`, and the port's own net becomes what the buffer's `O`
//     reads back. `bufif1 (probe_n, blink, ~button_n)` and VHDL's
//     `blink when oe else 'Z'` reach the same cell;
//   * **an enable whose sense the device file has to state.** A
//     `TRELLIS_IO`'s `T` is a *tristate* — a one releases the pad — where
//     an iCE40's `OUTPUT_ENABLE` is an *output enable* and a one drives
//     it. `src/fpga/devices/*.dev` says which with `oen=` or `oe=`, and
//     two of the four files had it the wrong way round until this design
//     needed it to be right;
//   * **`PIO<s>.BASE_TYPE = BIDIR_LVCMOS33`**, which is neither the
//     input's pattern nor the output's nor their union: on the top edge it
//     is eight bits where an output is six and an input five;
//   * **a tristate that is routed rather than tied.** The wire
//     `PADDTB_PIO` of this pad is driven by the route from the button on
//     the right edge, about thirty rows away, and the `CIB` mux that ties
//     it for every ordinary output is left alone. That is the same mux, so
//     tying it would have been a second driver on a wire a signal already
//     drives;
//   * **`PIO<s>.PULLMODE = UP`**, asked for by `-pullup yes` in the
//     constraints. On a pad that spends half its time released the pull is
//     the only thing deciding what it reads, and the field's default in
//     Project Trellis' database is a pull-*down*.
//
// Pins: testdata/fpga/cynthion/bidir_loopback.rcf.

module bidir_loopback (
    input  wire clk,
    input  wire button_n,

    // Ball E13, which is LED 0. Driven low while `USER` is held and
    // released the rest of the time; read back through the same pin.
    inout  wire probe_n,

    output wire read_low_n,   // LED 1: lit while E13 reads low
    output wire read_high_n,  // LED 2: lit while E13 reads high
    output wire heartbeat_n,  // LED 3: the counter bit itself
    output wire dark4_n,      // LED 4: dark, always
    output wire dark5_n       // LED 5: dark, always
);

    reg [25:0] count = 26'd0;

    // `toggle[i]` is the carry into bit i: one when every lower bit is
    // one, so `count ^ toggle` is `count + 1` exactly. Written as a
    // reduction rather than as a carry chain because `CCU2C` has no port
    // map in `src/fpga/devices/ecp5.dev`, and as a reduction rather than a
    // chain of ANDs so that mapping gets a balanced tree and not a 25-deep
    // ripple. `clock_blink.v` has the full argument; `tests/fpga_xray.rs`
    // proves this spelling equivalent to `count + 1` with Reticle's own
    // equivalence checker.
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

    // 0.89 Hz off the board's 60 MHz oscillator.
    wire blink = count[25];

    // THE TURNAROUND. `button_n` is low while the button is held, so the
    // enable is its inverse and the pad drives exactly while a finger is
    // on the button. Released, the pin's own pull-up decides what it reads,
    // which is why the constraints ask for one.
    assign probe_n = ~button_n ? blink : 1'bz;

    // THE WAY BACK IN. These two read the same pin the line above drives,
    // through the input buffer of the same pad, and they are complements,
    // so exactly one of the two LEDs is lit at every instant.
    assign read_low_n  =  probe_n;
    assign read_high_n = ~probe_n;

    // What the design meant, so that the pair above can be compared with
    // it: lit at the same instants the pad is driven low.
    assign heartbeat_n = blink;

    // Dark, always. A one is dark; see the header. Driven rather than left
    // alone so that what a person sees is "four of six moving" and not
    // "four of six moving and two unknown".
    assign dark4_n = 1'b1;
    assign dark5_n = 1'b1;

endmodule
