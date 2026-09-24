// A button driving an LED through one lookup table, on a Sipeed Tang
// Primer 20K (a Gowin GW2A-LV18PG256C8/I7 on its dock).
//
// The Gowin counterpart of `examples/basys3/sw_led.v`: the smallest
// design that exercises an input buffer, a LUT, an output buffer and the
// routing between them, and nothing clocked.
//
// WHAT A PERSON SHOULD SEE: pressing the dock's button S4 changes the
// state of the LED marked LED1, and nothing else on the board responds.
//
// WHAT HAS BEEN TRIED ON A PART: on 2026-09-24 this was synthesised,
// placed, routed and written as a `.fs` by `reticle fpga`, loaded into the
// part's SRAM by `reticle program`, which read DONE set, and a person
// pressed S4 and watched LED1 react. See `docs/fpga-gowin.md`.
module key_led (
    input  wire key,
    output wire led
);
    assign led = ~key;
endmodule
