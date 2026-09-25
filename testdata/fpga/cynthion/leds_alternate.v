// Every other one of a Cynthion's FPGA LEDs, lit.
//
// ===================================================================
// WHAT A PERSON LOOKING AT THE BOARD SHOULD SEE
// ===================================================================
//
// **LEDs 0, 2 and 4 lit and LEDs 1, 3 and 5 dark**, steadily, none of
// them blinking. Reading the row nearest the USB connectors from the one
// silkscreened `0`: on, off, on, off, on, off.
//
// This design exists because `leds.v`, which lights all six, is a
// slightly weaker observation on its own: a person seeing six lit LEDs
// has to take it on trust that they were not lit by something else. Six
// alternating ones are not a state anything else on this board produces.
// Between the two, the claim being made is that **each pad is configured
// individually and the value driven into it is the design's**.
//
// The LEDs are active low — see `leds.v` for the citation — so a zero in
// `led_n` is a lit LED. `6'b101010` therefore lights bits 0, 2 and 4,
// which are the LEDs silkscreened 0, 2 and 4: `led_n[0]` is ball E13,
// which is LED 0.
//
// Pins: testdata/fpga/cynthion/leds.rcf, the same file `leds.v` uses.

module leds_alternate (
    output [5:0] led_n
);

    // Bit 0 is LED 0, and a zero is lit: on, off, on, off, on, off.
    assign led_n = 6'b101010;

endmodule
