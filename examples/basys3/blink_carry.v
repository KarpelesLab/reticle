// A 32-bit counter off the board's oscillator, written the obvious way,
// with its top sixteen bits on the sixteen LEDs of a Digilent Basys 3.
//
// This is the design that exercises a 7-series carry chain. `count + 1`
// maps onto eight `CARRY4` primitives, one per four bits, chained carry
// out to carry in, and this flow places them up one column of slices and
// routes the carry through the slices' dedicated `COUT` -> `CIN` path.
// `blink.v` beside it spells the same kind of increment out as lookup
// tables, because it was written before a carry chain could be placed
// here; it is kept because it is the design that was watched working.
//
// WHAT A PERSON SHOULD SEE: the row of sixteen LEDs counting in binary,
// LED 0 (the right-hand end, U16) the least significant. Bit n of the
// counter changes every 2^n cycles of 100 MHz, so:
//
//   LED 15 (L1)   bit 31   one full period every 43 s
//   LED 12 (P3)   bit 28   every 5.4 s
//   LED  9 (V3)   bit 25   every 0.67 s, 1.49 Hz: the rate blink.v's LED 0 has
//   LED  7 (V14)  bit 23   about 6 Hz, a fast but countable flicker
//   LED  6 (U14)           dark; see below
//   LED  5 (U15)  bit 21   about 24 Hz, a shimmer
//   LEDs 0 to 4            too fast to see: each looks steadily half lit
//
// and each visible LED blinks at exactly half the rate of the LED to its
// right. That last sentence is the check. A carry that does not reach a
// bit makes that bit and everything above it stop or run at the wrong
// ratio; a chain broken between two `CARRY4`s shows as the LED for the
// bit just above a multiple of four (bit 24 is LED 8, bit 28 is LED 12)
// misbehaving while the LEDs to its right are right. A counter starting
// from the wrong value or a slice that reads an unrouted input as one
// shows as LEDs lit in a pattern that does not count.
//
// WHAT HAS BEEN TRIED ON A PART: nothing yet. The bitstream is built,
// fully routed, and every bit it sets decodes back into a feature the
// database names; `tests/fpga_xray_carry.rs` checks that. Nobody has
// loaded it into a board. `docs/fpga-xray.md` has the detail.
//
// LED 6 (U14) stays dark. Its ball is the one of the sixteen in a
// `LIOB33_SING` tile, which this flow's IO tables do not describe, so the
// design does not drive it rather than guess at its bits: the LEDs are two
// ports, `led_lo` for LEDs 0 to 5 and `led_hi` for LEDs 7 to 15, and
// counter bit 22 goes nowhere.
module blink_carry (
    input  wire       clk,
    output wire [5:0] led_lo,
    output wire [8:0] led_hi
);
    reg [31:0] count = 32'd0;

    always @(posedge clk) begin
        count <= count + 32'd1;
    end

    assign led_lo = count[21:16];
    assign led_hi = count[31:23];
endmodule
