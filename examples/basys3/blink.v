// A counter off the board's oscillator, blinking one LED, on a Digilent
// Basys 3.
//
// This is the smallest design that needs everything `sw_led` does not: a
// clock arriving from a pad, a global clock buffer, the clock tree that
// carries it across the die, and flip-flops. `sw_led` has none of those —
// it is one lookup table between two pins — which is why it was the first
// milestone and this is the second.
//
// WHAT A PERSON SHOULD SEE: LED 0, at the right-hand end of the row of
// sixteen, blinking steadily about three times every two seconds, evenly
// on and off. Every other LED stays dark. Nothing else on the board
// does anything, and no switch or button has any effect.
//
// The rate: the oscillator is 100 MHz, so bit 25 of the counter changes
// every 2^25 cycles, which is 0.336 s. On for a third of a second, off
// for a third of a second, or 1.49 Hz. Slow enough to count by eye and
// fast enough that a dead design is obvious rather than ambiguous.
//
// WHAT HAS BEEN TRIED ON A PART: built by this flow, fully routed, and
// loaded into a Basys 3 by `reticle program`, which reported `DONE` high
// with no CRC error. **Nobody has yet watched the LED**, so the line
// above is what the design should do and not a report of what it did.
// `docs/fpga-xray.md` says the same in more detail.
//
// # Why the increment is spelled out
//
// `count <= count + 1` is the obvious way to write this, and Reticle maps
// it — correctly — onto seven `CARRY4` primitives. A 7-series carry chain
// cannot be *routed* by this flow yet, and the reason is structural
// rather than a missing table: a `CARRY4`'s `S` inputs are wired inside
// the slice to the four lookup tables' `O6` outputs and reach no tile
// wire at all, so every bit of the propagate needs a lookup table in the
// same slice at the same position, and the placer has no way to say
// "these two cells share a slice". The chain also has to run up one
// column of slices, because `CIN` comes only from the `COUT` of the slice
// below. Mapping is not the gap; packing and chain placement are.
//
// So the increment is written as what a carry chain computes: bit i
// toggles when every bit below it is one. That is `count + 1` exactly,
// and not by assertion: `tests/fpga_xray.rs` synthesises this module and
// a `count <= count + 1` twin and has Reticle's own equivalence checker
// prove the two the same, which it does inductively rather than for a
// handful of cycles. The result maps to 64 six-input lookup tables three
// deep, which the interconnect routes. Three levels is not a timing risk
// at 10 ns, but nothing here has measured it: Reticle has no 7-series
// delay model, so `reticle timing` on this design reports placeholders.
// A reduction is used rather than a chain of ANDs for that reason — a
// 25-deep ripple would be a real risk at 100 MHz and would look the same
// in every report this flow can produce.
module blink (
    input  wire clk,
    output wire led
);
    reg [25:0] count = 26'd0;

    // `toggle[i]` is the carry into bit i: one when every lower bit is
    // one. Written as a reduction rather than a chain of ANDs so that
    // mapping gets a balanced tree and not a 25-deep ripple.
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

    assign led = count[25];
endmodule
