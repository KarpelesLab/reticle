// All six of a Cynthion's FPGA LEDs, lit.
//
// ===================================================================
// WHAT A PERSON LOOKING AT THE BOARD SHOULD SEE
// ===================================================================
//
// **All six LEDs numbered 0 to 5 lit, steadily, at the same brightness,
// and none of them blinking.** They are the row nearest the USB
// connectors, silkscreened 0 1 2 3 4 5, and they are the FPGA's.
//
// The five LEDs lettered A to E are the debug microcontroller's and this
// design does not touch them. Whatever they were doing, they carry on
// doing; in particular B and D say what the microcontroller thinks about
// the FPGA and the USB port, not what this design is doing.
//
// The observation is "dark, then all six on", and it is not something the
// board does on its own. But **`DONE` being high is not the observation**,
// and that is worth saying here because it has already misled once: on
// 2026-09-25 this design was loaded, the part asserted `DONE` with no
// fault bit, and on 2026-09-26 the board's owner reported all six LEDs
// dark. What was missing was the IO bank's `BANK.VCCIO` setting, which
// lives in a tile none of these pads owns; `docs/fpga-trellis.md` has the
// finding and what checks it now. So a person looking at the board is
// still the only thing that settles this, which is why this header exists.
//
// ===================================================================
// WHY IT DRIVES ZERO TO LIGHT AN LED
// ===================================================================
//
// The LEDs are **active low**. Each anode is tied to +3V3 and each
// cathode goes through a resistor to the FPGA, so a pin driven *low*
// sinks the current and lights it, and a pin driven high leaves it dark.
// That is from Great Scott Gadgets' own hardware: `invert=True` on the
// `LEDResources` line of `cynthion_r1_4.py`, and the schematic sheet
// `indicators_buttons.kicad_sch` at tag `r1.4.0`, where all six anodes
// sit on one wire up to the `+3V3` symbol.
//
// Nothing in Reticle knows about that polarity — a device file describes
// a part, not a board — so this design drives the pads to zero and says
// here that zero is lit. The port is called `led_n` for the same reason.
//
// ===================================================================
// WHAT IT EXERCISES, AND WHAT IT DOES NOT
// ===================================================================
//
// Six output pads driven by a constant. There is nothing to route: no
// net has both a driver and a sink, which is why this design and not a
// counter is what the ECP5 backend can build today.
// `src/fpga/trellis/mod.rs` says why — that backend declares the part's
// geometry and its pads and **no interconnect at all** — and
// `TrellisFabric::unroutable` is the check that stops a design with
// anything to route from being turned into a bitstream that could not
// work.
//
// So this proves the pads, the placer's pin constraints, the frame map,
// the container and the configuration sequence, and it proves nothing
// about routing or about logic. `docs/fpga-trellis.md` keeps that
// distinction.
//
// Pins: testdata/fpga/cynthion/leds.rcf.

module leds (
    output [5:0] led_n
);

    // Zero is lit. See the header.
    assign led_n = 6'b000000;

endmodule
