// A counter off the board's oscillator, blinking one LED, on a Digilent
// Basys 3.
//
// This is the smallest design that needs everything `sw_led` does not: a
// clock arriving from a pad, a global clock buffer, the clock tree that
// carries it across the die, flip-flops, and the carry chain the
// increment maps onto. `sw_led` has none of those — it is one lookup
// table between two pins — which is why it was the first milestone and
// this is the second.
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
// THIS DOES NOT ROUTE YET, so nothing built from it has been loaded into
// a part and it would configure almost nothing if it were. The flow maps
// it correctly — one global buffer, twenty-six flip-flops, seven carry
// elements, every bel pin resolved — and then places 4 of 55 signals,
// because routing a clock across the die needs resources the loader does
// not yet describe. `reticle fpga` says so on every run rather than
// writing a bitstream that looks finished.
//
// `sw_led` next door is the design that has run on a part. See
// `docs/fpga-xray.md` for the difference.
module blink (
    input  wire clk,
    output wire led
);
    reg [25:0] count = 26'd0;

    always @(posedge clk) begin
        count <= count + 26'd1;
    end

    assign led = count[25];
endmodule
