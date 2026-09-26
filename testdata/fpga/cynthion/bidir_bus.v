// An eight-bit bidirectional bus on a Cynthion's RIGHT edge — the
// auxiliary ULPI transceiver's own data balls — driven, released, and read
// back through the same eight pins, with the result latched onto two LEDs.
//
// `bidir_loopback.v` turned one pin around on the top edge. This turns
// eight around on the right one, which is the edge a ULPI bus is on and the
// edge where four PIOs share a pad tile. Nothing about the pad was in the
// way; reading the bitstream back was. See "What could not be read back" in
// `docs/fpga-trellis.md`.
//
// ===================================================================
// WHAT A PERSON SHOULD PRESS, AND WHAT THEY SHOULD SEE
// ===================================================================
//
// **Press and hold the button silkscreened `USER` for three seconds, and
// watch LED 1 come on and stay on while LED 2 never does.** The other two
// tactile buttons end the experiment rather than perform it: `PROG` makes
// the FPGA reload itself from flash, and `RESET` resets the debug
// microcontroller.
//
// The six FPGA LEDs are the row beside the silkscreen legend `FPGA LEDs`.
// They are not numbered on the board, so they are counted from the end of
// the row FARTHEST from the USER button, which is `led_n[0]`, the diode
// `D7`.
//
// With this design in the part and **nobody touching the board**:
//
//     LED 0   dark                     the bus is released
//     LED 1   dark                     nothing has been read back yet
//     LED 2   dark                     and nothing has disagreed
//     LED 3   blinking, 0.89 Hz        the clock runs
//     LED 4   LIT, steady              the transceiver has let go of the bus
//     LED 5   dark                     it is not claiming it right now
//
// While `USER` is **held**, and then after it is let go:
//
//     LED 0   LIT while held           the FPGA is driving all eight pins
//     LED 1   comes on within ~1.2 s, AND STAYS ON afterwards
//     LED 2   stays dark, for good
//     LED 3   still blinking
//     LED 4   still lit
//     LED 5   dark
//
// **LED 1 is the milestone.** It latches high the first clock edge on which
// all eight pins read back exactly the eight bits the pads are driving.
// LED 2 latches high if any pin ever reads back something else. They are
// latches and not levels, so what a person sees a second later is what
// happened, not what is happening: **LED 1 on and LED 2 off is the whole
// claim**, and it cannot be produced by a glimpse.
//
// LED 4 and LED 5 are about the other end of the bus and are explained
// under "WHY THIS IS SAFE" below. LED 4 lit is the precondition for the
// test running at all.
//
// **What would be wrong, and what it would look like:**
//
//   * **LED 4 dark, LED 0/1/2 dark** — the transceiver never let go of the
//     bus, so the pads were never asked to drive and nothing was measured.
//     Most likely the 60 MHz is not reaching its clock ball (D16): an
//     unclocked ULPI transceiver holds `dir` asserted on purpose, to
//     protect its own inputs (ULPI 1.1 §3.12).
//   * **LED 0 lit, LED 1 and LED 2 both dark** — the pads drive and the
//     comparison never runs. Nothing but a stopped counter does that, and
//     LED 3 would be frozen too.
//   * **LED 2 lit** — at least one pin did not read back what it drove.
//     That is the interesting failure: the output path or the input path of
//     one pad of the bus is wrong. LED 1 lit as well means intermittent.
//   * **LED 0 dark while `USER` is held** — either the button is not being
//     read, or the transceiver is claiming the bus at that moment (LED 5
//     would be lit).
//   * **LED 3 frozen** — the clock stopped, and nothing else is meaningful.
//
// **At what rate.** LED 3 is on 0.56 s and off 0.56 s, 0.89 Hz, from bit 25
// of a counter on the board's 60.000 MHz oscillator. The eight pins are
// driven with a walking pattern that changes every 70 ms and takes 1.12 s
// to put both a one and a zero on every one of the eight, which is why the
// button wants holding for three seconds and not for one.
//
// ===================================================================
// WHY THIS IS SAFE, ON A BUS THAT HAS ANOTHER DRIVER
// ===================================================================
//
// These eight balls are **not** like `bidir_loopback.v`'s E13, whose net
// holds nothing but an LED and a resistor. They go to a USB transceiver
// chip, which drives them itself. Driving a pin something else is driving
// can damage hardware, so here is why this does not.
//
// 1. **This is the bus's own arbitration, not a workaround.** A ULPI data
//    bus is owned by the transceiver while `dir` is asserted and by the
//    FPGA otherwise (ULPI 1.1 §3.3). This design releases all eight pins
//    whenever `ulpi_dir` is high — combinationally, with no register in the
//    way, so the release is as fast as the pad can make it — and drives
//    them only while `dir` is low *and* a finger is on the USER button. So
//    the two drivers are interlocked by the signal the transceiver provides
//    for exactly that purpose, and with nobody at the board the FPGA never
//    drives these pins at all.
// 2. **The transceiver is told to ignore what is on the bus.** `stp` is
//    held **high** throughout. A ULPI transceiver "must stop interpreting
//    `data`" while `stp` is high unexpectedly — that is the specification's
//    own protection for a Link that is not driving the bus properly yet
//    (§3.12). So the walking pattern is not read as a transmit command, a
//    register write, or anything else: it goes nowhere.
// 3. **The transceiver is held out of reset and given its clock**, which is
//    what the board asks for: `clk_dir='o'` in Great Scott Gadgets' own
//    platform file means the FPGA drives the 60 MHz to the transceiver's
//    clock ball, and `rst_invert=True` means its reset is active low at the
//    ball. Both are done here. Out of reset and clocked, the transceiver
//    idles with `dir` low, which is what LED 4 reports.
// 4. **Great Scott Gadgets' own gateware drives these same eight balls**,
//    as bidirectional pins, in every bitstream they ship for this board:
//    `analyzer.bit` has all eight as `BIDIR_LVCMOS33`, and
//    `tests/fpga_trellis.rs` reads that back out of their file bit by bit.
//    `default_usb_connection = "aux_phy"` in the platform file means this
//    is the port their own designs put a USB device on.
// 5. **Nothing else on the board is touched.** The Type-C controllers, the
//    VBUS switches and the pseudo-supply pins are left alone, which is what
//    every gateware in the Cynthion repository does with them too — those
//    resources are declared in the platform file and used by none of it.
//    The `CONTROL` port, which is where the Apollo debugger this board is
//    reached over lives, is a different transceiver on different balls and
//    is not mentioned here.
//
// The balls, from `cynthion/python/src/gateware/platform/cynthion_r1_4.py`:
//
//     ULPIResource("aux_phy", 0,
//         data="F16 G15 G16 H15 J15 J16 K15 K16", clk="D16", clk_dir='o',
//         dir="E16", nxt="F15", stp="E15", rst="J13", rst_invert=True,
//         attrs=Attrs(IO_TYPE="LVCMOS33", SLEWRATE="FAST")),
//
// `SLEWRATE="FAST"` is the one attribute of that line this backend does not
// yet write; see "What remains" in `docs/fpga-trellis.md`. It changes an
// edge rate, not a direction, and nothing here depends on it: the walking
// pattern is static for four million clocks at a time.
//
// `nxt` (F15) is deliberately not in this design. It is an input and
// reading it would cost nothing, but it says nothing either — it is the
// transceiver's throttle for a transfer this design never starts.
//
// ===================================================================
// WHAT IT EXERCISES
// ===================================================================
//
//   * **eight bidirectional pads on the right edge**, where four PIOs share
//     one pad tile and a pseudo-differential `PIO<s>.BASE_TYPE` reaches
//     across a pair. Until the bitstream decoder learnt to resolve a field
//     by the reading that leaves fewest bits unexplained, this design was
//     refused: eight bits, two per pair, belonged to no feature and
//     `reticle fpga --bitstream` will not write that;
//   * **a clock leaving the FPGA**: the 60 MHz oscillator's own net reaches
//     a pad's output as well as the global clock network;
//   * **a released level that comes from the far end**, not from a pull-up.
//     The pads here ask for no pull at all, because the transceiver's input
//     pins and the board's traces are what is on the net;
//   * **a wide tri-state**: one enable drives eight buffers, from a
//     conditional assignment over an eight-bit vector.
//
// Pins: testdata/fpga/cynthion/bidir_bus.rcf.

module bidir_bus #(
    // Which bit of the counter advances the walking pattern. 22 is 70 ms a
    // step off 60 MHz, so a full sweep — eight pins, then the same eight
    // inverted — takes 1.12 s, which is what "hold it for three seconds"
    // in the header is for.
    //
    // It is a parameter for one reason: a testbench cannot wait 1.12 s of
    // simulated time, and the claim worth checking before a person is asked
    // to look is that **all eight** pads drive a one and a zero and read
    // both back. `bidir_bus_tb.v` sets this to 4, which makes a full sweep
    // 256 clocks, and checks exactly that. The board gets the default.
    parameter integer WALK = 22
) (
    input  wire clk,        // A8, the 60.000 MHz oscillator
    input  wire button_n,   // M14, low while `USER` is held

    // The auxiliary transceiver's eight data balls, all on the right edge.
    inout  wire [7:0] ulpi_data,

    input  wire ulpi_dir,    // E16: high while the transceiver owns the bus
    output wire ulpi_clk,    // D16: the 60 MHz, which the board asks the
                             //      FPGA to provide
    output wire ulpi_stp,    // E15: held high, so the transceiver ignores
                             //      whatever this design puts on `data`
    output wire ulpi_rst_n,  // J13: high, so the transceiver runs

    output wire led0_n,      // LED 0: driving all eight pins right now
    output wire led1_n,      // LED 1: latched — every pin read back what it
                             //        was driving
    output wire led2_n,      // LED 2: latched — some pin did not
    output wire led3_n,      // LED 3: heartbeat
    output wire led4_n,      // LED 4: latched — the transceiver has let go
    output wire led5_n       // LED 5: the transceiver is claiming the bus
);

    // ---------------------------------------------------------------
    // The time base, 0.89 Hz off 60 MHz, as `clock_blink.v` builds it
    // ---------------------------------------------------------------
    //
    // `toggle[i]` is the carry into bit i: one when every lower bit is one,
    // so `count ^ toggle` is `count + 1` exactly. Written as a reduction
    // rather than as an adder because `CCU2C` has no port map in
    // `src/fpga/devices/ecp5.dev`, and as a reduction rather than a chain
    // of ANDs so that mapping gets a balanced tree and not a 25-deep
    // ripple. `tests/fpga_xray.rs` proves this spelling equivalent to
    // `count + 1` with Reticle's own equivalence checker.
    reg [25:0] count = 26'd0;
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

    // ---------------------------------------------------------------
    // The pattern the eight pins are driven with
    // ---------------------------------------------------------------
    //
    // A walking one, and then a walking zero: `count[WALK+2:WALK]` picks
    // the pin, `count[WALK+3]` picks the polarity. So over 1.12 s every one
    // of the eight pads drives a high and a low, and in each half exactly
    // one of the eight differs from the other seven. What that catches that
    // a constant would not: a pad that can drive one level and not the
    // other, and two pins shorted together — the pair would read each
    // other's level in the half where they disagree.
    //
    // What it does **not** catch, and no self-loopback can: two balls
    // swapped in the constraints. A pad reads its own pin, so the design
    // sees what it drove whatever ball that pin is on, and the eight `set_io`
    // lines are checked by reading them, not by this. Written as a case
    // rather than as `8'b1 << count[WALK+2:WALK]` so that what reaches the
    // pads is eight constants and not a shifter.
    wire [2:0] phase = count[WALK+2:WALK];

    reg [7:0] onehot;
    always @* begin
        case (phase)
            3'd0:    onehot = 8'b0000_0001;
            3'd1:    onehot = 8'b0000_0010;
            3'd2:    onehot = 8'b0000_0100;
            3'd3:    onehot = 8'b0000_1000;
            3'd4:    onehot = 8'b0001_0000;
            3'd5:    onehot = 8'b0010_0000;
            3'd6:    onehot = 8'b0100_0000;
            default: onehot = 8'b1000_0000;
        endcase
    end

    wire [7:0] pattern = count[WALK+3] ? ~onehot : onehot;

    // ---------------------------------------------------------------
    // THE TURNAROUND
    // ---------------------------------------------------------------
    //
    // Driven only while a finger is on the button AND the transceiver is
    // not claiming the bus. Both conditions are combinational, so the pads
    // let go of the bus in the same cycle `dir` rises: there is no register
    // between `ulpi_dir` and the eight enables.
    wire driving = ~button_n & ~ulpi_dir;

    assign ulpi_data = driving ? pattern : 8'bz;

    // ---------------------------------------------------------------
    // THE WAY BACK IN
    // ---------------------------------------------------------------
    //
    // `ulpi_data` read here is the *pin*, through the input buffer of the
    // same eight pads: `fpga::primitives`' IO pass re-drives the port's own
    // net from what each buffer reads off its ball, so everything in the
    // design that reads the port reads the ball.
    //
    // The comparison is gated three ways, and each gate is there for a
    // reason a failure would otherwise be blamed on the pads:
    //
    //   * `driving` delayed by two clocks, so the sample is never taken in
    //     the cycle the enables turned on or the cycle after it;
    //   * `|count[WALK-1:2]`, which is low for the four clocks after the
    //     pattern changes, so the sample is never taken while a pin is
    //     still moving. Four clocks is 67 ns, and the pattern then holds
    //     for 70 ms;
    //   * `driving` itself, so nothing is sampled while the bus belongs to
    //     the transceiver.
    reg drive_d1 = 1'b0;
    reg drive_d2 = 1'b0;
    always @(posedge clk) begin
        drive_d1 <= driving;
        drive_d2 <= drive_d1;
    end

    wire settled = |count[WALK-1:2];
    wire compare = driving & drive_d1 & drive_d2 & settled;
    wire agree   = ulpi_data == pattern;

    // Latches, not levels: an LED showing a level would be showing whether
    // a finger is on the button. These show what happened.
    //
    // Written as `q <= q | event` rather than as `if (event) q <= 1'b1`,
    // which means the same thing, because the second spelling infers a
    // flip-flop with a **clock enable** and the first does not. A slice's
    // two flip-flops share one `CE` wire, so three latches with three
    // different enables are three constraints on where the placer may put
    // them — a constraint `fpga::place` does not know about. That is the
    // obstacle the eight-bit ULPI device core is currently stuck on (see
    // "What remains" in `docs/fpga-trellis.md`), and there is no reason for
    // this design to depend on it.
    //
    // All three come up at zero, so both LEDs start dark: an ECP5's
    // flip-flops are released into their `REGSET` state at the end of
    // configuration, and `src/fpga/devices/ecp5.dev` gives every one of
    // these `REGSET="RESET"`.
    reg saw_match = 1'b0;
    reg saw_miss  = 1'b0;
    reg saw_free  = 1'b0;
    always @(posedge clk) begin
        saw_match <= saw_match | (compare &  agree);
        saw_miss  <= saw_miss  | (compare & ~agree);
        saw_free  <= saw_free  | ~ulpi_dir;
    end

    // ---------------------------------------------------------------
    // The other end of the bus, and the LEDs
    // ---------------------------------------------------------------
    //
    // The transceiver gets the clock the board says the FPGA owes it, is
    // held out of reset, and is told to ignore the data bus. `stp` high is
    // ULPI's own interface protection and is the reason the walking pattern
    // is not a stream of commands; see "WHY THIS IS SAFE" above.
    assign ulpi_clk   = clk;
    assign ulpi_rst_n = 1'b1;
    assign ulpi_stp   = 1'b1;

    // Active low: the LED anodes are on +3V3 and the cathodes on the FPGA,
    // so a pin driven low lights one.
    assign led0_n = ~driving;
    assign led1_n = ~saw_match;
    assign led2_n = ~saw_miss;
    assign led3_n = ~count[25];
    assign led4_n = ~saw_free;
    assign led5_n = ~ulpi_dir;

endmodule
