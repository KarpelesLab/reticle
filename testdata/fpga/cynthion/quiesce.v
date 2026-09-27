// Put a Cynthion's auxiliary USB transceiver back to sleep, and say so with
// one LED.
//
// ===================================================================
// WHY THIS EXISTS
// ===================================================================
//
// **A transceiver's registers outlive the FPGA's configuration.** On this
// board the 1.5 kOhm pull-up that tells a host a device is attached is not
// a pin the FPGA drives: it is `TermSelect` in the transceiver's Function
// Control register, written over the ULPI bus. Reconfiguring the FPGA away
// from a design that wrote that bit does **not** clear it. The transceiver
// keeps the pull-up connected, the host keeps seeing a device that cannot
// answer, and a machine's kernel log fills with
//
//     usb 7-5: device not accepting address NN, error -71
//
// every few seconds, indefinitely. `docs/fpga-trellis.md`'s "Where it
// stops, and what was ruled out" is where that was found out and written
// down; this file is the cure it names, committed so that it is not rebuilt
// by hand the next time.
//
// The cure is the transceiver's own reset **pin**, which is outside ULPI and
// which the FPGA does drive: `J13`, active low at the ball
// (`rst_invert=True` in Great Scott Gadgets' platform file). Holding it at
// zero resets the whole part — the interface, the register set and the
// analogue front end — so Function Control goes back to its `41h` default,
// `TermSelect` clear, no pull-up, and the host sees the port empty and stops
// retrying. Nothing else in this design does anything, which is the point:
// there is no clock, no state and no bus.
//
// Load this, watch the kernel log go quiet, and then load whatever was
// wanted. A power cycle reloads the part from the board's flash instead,
// which nothing in this project writes.
//
// ===================================================================
// WHAT A PERSON SEES
// ===================================================================
//
// **LED 5 alone, lit and steady — the one at the end nearest the button
// silkscreened `USER`.** That is deliberately the opposite end of the row
// from every other design in this directory, all of which start at LED 0
// (`E13`, the far end): a board holding this one is visibly not holding an
// experiment. The other five are driven high, so they are dark because
// something is driving them dark rather than because nothing is driving
// them at all.
//
// There is no heartbeat to look for. A design with no clock cannot have
// one, and "steady" is the whole claim: if LED 5 is lit, `J13` is being
// held low by the same configuration.
//
// ===================================================================
// WHAT IS NOT HERE, AND WHY
// ===================================================================
//
//   * **The eight data balls** (`F16 G15 G16 H15 J15 J16 K15 K16`). They are
//     left out of the design entirely, so the FPGA's pads for them are
//     unconfigured and drive nothing. A design that is holding the
//     transceiver in reset has nothing to say to it, and a pad that is not
//     in the design cannot fight a transceiver that comes out of reset for
//     some other reason.
//   * **`stp` (`E15`)**, for the same reason and one better: ULPI 1.1 §3.12
//     gives a transceiver a *weak pull-up* on `stp` and requires it to stop
//     interpreting `data` while `stp` is unexpectedly high. An FPGA pad that
//     drives nothing therefore leaves `stp` in its protective state, which is
//     a better thing to leave behind than either level. `usb_ulpi_device.rcf`
//     does constrain `E15`, so this is a choice and not a limitation.
//   * **`dir` and `nxt`** (`E16`, `F15`), which are the transceiver's outputs.
//     Nothing here reads them.
//   * **The interface clock** (`D16`). `clk_dir='o'` means the FPGA owes the
//     transceiver 60 MHz when the transceiver is running. It is not running.
//     An unclocked ULPI transceiver asserts `dir` and protects its own data
//     inputs (§3.12), and one held in reset does not even do that.
//   * **The `USER` button.** There is nothing to start or stop.
//
// Pins: testdata/fpga/cynthion/quiesce.rcf.

module quiesce (
    // J13, the auxiliary transceiver's reset, active low at the ball.
    // Zero holds the transceiver in reset: no pull-up, no attachment, and
    // whatever the last design wrote to its registers forgotten.
    output wire ulpi_rst_n,

    // The six FPGA LEDs, active low. LED 5 is the end nearest `USER`.
    output wire led0_n,
    output wire led1_n,
    output wire led2_n,
    output wire led3_n,
    output wire led4_n,
    output wire led5_n
);

    assign ulpi_rst_n = 1'b0;

    assign led0_n = 1'b1;
    assign led1_n = 1'b1;
    assign led2_n = 1'b1;
    assign led3_n = 1'b1;
    assign led4_n = 1'b1;
    assign led5_n = 1'b0;

endmodule
