// A Cynthion's USER button, through the fabric and a lookup table, to two
// of its LEDs.
//
// ===================================================================
// WHAT A PERSON SHOULD PRESS, AND WHAT THEY SHOULD SEE
// ===================================================================
//
// **Press and hold the button silkscreened `USER`.** The board has three
// tactile buttons and the other two will end the experiment rather than
// perform it: `PROG` (or `PROGRAM`) makes the FPGA reload itself from
// flash, and `RESET` resets the debug microcontroller. On the r1.4.0
// layout `USER` and `PROG` are on the same edge of the board and `RESET`
// is on the opposite one (`SW3`, `SW1` and `SW2` of `cynthion.kicad_pcb`).
//
// **Which LEDs to watch.** The six FPGA LEDs are the row of six beside the
// silkscreen legend `FPGA LEDs`. They are **not** individually numbered on
// the board — the r1.4.0 silkscreen has that one legend and no digits — so
// the two to watch are identified by geometry instead:
//
//     the two at the end of the row FARTHEST from the USER button.
//
// That is `led_n[0]` (the very end) and `led_n[1]` (next to it). On the
// r1.4.0 layout `led_n[0]` is the diode `D7` and `led_n[5]` is `D2`, and
// `D7` is the end of the row away from the button; the five LEDs lettered
// A to E elsewhere on the board are the debug microcontroller's and this
// design does not touch them.
//
// With this design in the part and nobody touching the board:
//
//     end farthest from the button:  DARK, then LIT
//     the other four:                dark
//
// While the `USER` button is held down:
//
//     end farthest from the button:  LIT, then dark
//     the other four:                dark
//
// and when it is let go they swap back. The two are always in opposition,
// never both lit and never both dark, and the other four never light.
//
// **If the two that change are at the end NEAREST the button instead**,
// everything here works and the LED numbering is the other way round from
// what Great Scott Gadgets' schematic implies. That is worth reporting: it
// is the one thing about this board `docs/fpga-trellis.md` has never been
// able to settle from a file.
//
// The observation is deliberately a strong one:
//
//   * **one** LED lit rather than all six, so it cannot be confused with
//     the previous milestone (`leds.v`, which lit all six);
//   * the lit one **moves** when a finger moves, which no configuration
//     that does nothing can do;
//   * and it moves **both ways**, so a stuck input shows up as one of the
//     two LEDs never changing rather than as nothing at all.
//
// ===================================================================
// WHY IT IS SPELLED THIS WAY ROUND
// ===================================================================
//
// **Both signals are active low, and they cancel out on LED 0.**
//
// The LEDs have their anodes on +3V3 and their cathodes on the FPGA
// through a series resistor, so a pin driven *low* lights one. The button
// has a 10 k pull-up to +3V3 and shorts to ground when pressed, so the
// ball reads *low* while it is held. `leds.v`'s header and
// `docs/fpga-trellis.md` have the citations for both.
//
// So `led_n[0] = button_n` lights LED 0 exactly while the button is held,
// and `led_n[1] = ~button_n` lights LED 1 exactly while it is not. Ports
// named `_n` for both.
//
// ===================================================================
// WHAT IT EXERCISES
// ===================================================================
//
// This is the first ECP5 design of this project with anything to route.
// It compiles to seven IO buffers and one `LUT4`, and it needs:
//
//   * an **input** pad, which the six-LED milestone did not have: a
//     different `BASE_TYPE`, plus `HYSTERESIS` and `PULLMODE` settings an
//     output does not need. `PULLMODE=NONE` is not cosmetic — the
//     database's default for the field is an internal pull-*down*, which
//     would fight this board's pull-up through its 33 k series resistor
//     and hold the pin low whether or not anybody pressed anything;
//   * a pad on the **right** edge of the die, where four PIOs share a
//     position and the configuration is one row south — the top edge's
//     rule does not apply and getting it wrong is invisible;
//   * a **lookup table**, whose truth table is the one thing on this
//     family whose bits depend on which of its inputs the router reached;
//   * and **interconnect**: two signals, one of which fans out to a LED
//     pad and to the LUT, crossing about thirty rows of the die from the
//     button on the right edge to the LEDs on the top.
//
// It needs **no clock**, which keeps the one thing this backend still
// cannot build out of the way: there is no clock network here, so nothing
// sequential can be placed.
//
// LEDs 2 to 5 are driven high — dark — rather than left alone, so that
// what a person sees is "one of six", and because a pad tied to a
// constant and a pad driven by a route are configured differently and
// this exercises both in one bitstream.
//
// Pins: testdata/fpga/cynthion/button_led.rcf.

module button_led (
    input  wire       button_n,
    output wire [5:0] led_n
);

    // Lit while the button is held: both signals are active low, so the
    // two inversions cancel and this is a buffer.
    assign led_n[0] = button_n;

    // Lit while it is not. This is the lookup table.
    assign led_n[1] = ~button_n;

    // Dark, always. A one is dark; see the header.
    assign led_n[5:2] = 4'b1111;

endmodule
