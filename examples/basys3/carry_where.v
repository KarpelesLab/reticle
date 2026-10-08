// Where, if anywhere, does a 7-series carry chain disagree with logic
// that uses no carry at all?
//
// `carry_probe.v` beside this one answers "does it disagree", and that
// turned out not to be a question a person at a board can answer: its
// single latched LED closed for three different reasons over one
// afternoon — a stuck pad, a welded routing join, and finally the two
// counters being released from configuration one clock edge apart — and
// each time the lit LED looked identical. This design answers "where",
// which is a question whose answer carries its own diagnosis.
//
// ===================================================================
// WHAT A PERSON SHOULD SEE, AND WHAT THEY ARE NEVER ASKED TO JUDGE
// ===================================================================
//
// Two LEDs blink, slowly, about three times every two seconds, and in
// step with each other. Everything else is lit or dark and never moves.
//
//   LD10  the carry-FREE counter's bit 25       BLINKS ~1.5 Hz
//   LD14  the carry-CHAIN counter's bit 25      BLINKS ~1.5 Hz, in step
//   LD5   ever differed in bits  0..5           DARK
//   LD7   ever differed in bits  6..12          DARK
//   LD8   ever differed in bits 13..18          DARK
//   LD9   ever differed in bits 19..25          DARK
//   LD11  a constant zero                       DARK  <- the control
//
// **Nothing here flickers.** An earlier design put a 12 Hz signal on an
// LED and asked whether it was blinking; the honest answer from the
// person at the board was "lit", and that reading was then used as data.
// A rate fast enough to need judging is not an instrument. So the only
// moving lights are at 1.5 Hz, which is countable by eye, and every
// other claim is a latch that is simply on or off.
//
// ===================================================================
// HOW TO READ IT
// ===================================================================
//
// - **Both blink in step, all four dark.** The `CARRY4` chain computes
//   plus one, across all 26 bits, for the whole 2^26 count. Bit 25 can
//   only move if every carry below it propagates, and the four latches
//   say the two counters were never once unequal.
// - **Both blink, one or more of LD5/7/8/9 lit.** A real disagreement,
//   localised. Which one is lit says which four-bit group the carry
//   fails to cross, and `CARRY4` covers four bits per cell, so the
//   lowest lit lamp points at the cell boundary that does not carry.
// - **LD14 dark or steady while LD10 blinks.** The carry chain does not
//   reach bit 25 at all, which is a broken chain rather than a wrong sum.
// - **LD10 dark or steady.** The carry-free reference is broken, so
//   nothing else on the board means anything. This is the control that
//   earlier designs lacked.
// - **LD11 lit.** The instrument is lying; discard the whole reading.
//
// ===================================================================
// ARMING, WHICH IS WHERE THE LAST ATTEMPT WENT WRONG
// ===================================================================
//
// Both counters are held at zero until `armed` and released on **one**
// edge, so that a ragged release from configuration cannot leave them
// permanently one apart — which would latch a difference that says
// nothing about the carry chain.
//
// `arm` shifts **ones** in and therefore saturates: once bit 15 is set
// it can never clear. The previous attempt walked a single one up a
// shift register under an enable, which can walk off the end — and did:
// `arm` reached zero, `armed` went low forever, both counters froze at
// zero, and the two latches stayed lit from the brief window while it
// had been armed. A one-shot that can disarm itself is worse than the
// problem it was added to solve.
//
// No adder is used anywhere except the chain counter itself, which is
// the subject of the experiment. The arm is a shift register and the
// reference counter is `blink.v`'s idiom.

module carry_where #(
    // The counters' width. 26 bits off 100 MHz puts the top bit at
    // 1.49 Hz. A testbench uses a small width to run the same RTL in a
    // few thousand cycles; the four groups below are quarters of it, so
    // the comparison keeps its shape at any width.
    parameter WIDTH = 26,
    // How many edges the arm waits before releasing both counters.
    parameter ARM = 16
) (
    input  wire clk,
    output wire led_free,      // LD10: the carry-free counter, 1.5 Hz
    output wire led_chain,     // LD14: the carry-chain counter, 1.5 Hz
    output wire led_diff_lo,   // LD5:  bits  0..6
    output wire led_diff_mid,  // LD7:  bits  7..12
    output wire led_diff_hi,   // LD8:  bits 13..18
    output wire led_diff_top,  // LD9:  bits 19..25
    output wire led_zero       // LD11: the control
);
    // Quarters of the width, so the four latches below partition every
    // bit of the comparison with none left out.
    localparam integer G1 = WIDTH / 4;
    localparam integer G2 = (2 * WIDTH) / 4;
    localparam integer G3 = (3 * WIDTH) / 4;

    // Saturating: ones shift in, so `armed` latches high for good.
    reg [ARM-1:0] arm = {ARM{1'b0}};
    always @(posedge clk) arm <= {arm[ARM-2:0], 1'b1};
    wire armed = arm[ARM-1];

    // The reference: no CARRY4, by construction.
    reg [WIDTH-1:0] free = {WIDTH{1'b0}};
    wire [WIDTH-1:0] toggle;
    assign toggle[0] = 1'b1;
    genvar i;
    generate
        for (i = 1; i < WIDTH; i = i + 1) begin : lanes
            assign toggle[i] = &free[i-1:0];
        end
    endgenerate

    // The subject: the obvious spelling, which maps onto one CARRY4 per
    // four bits.
    reg [WIDTH-1:0] chain = {WIDTH{1'b0}};

    always @(posedge clk) begin
        if (!armed) begin
            free  <= {WIDTH{1'b0}};
            chain <= {WIDTH{1'b0}};
        end else begin
            free  <= free ^ toggle;
            chain <= chain + 1'b1;
        end
    end

    reg diff_lo = 1'b0, diff_mid = 1'b0, diff_hi = 1'b0, diff_top = 1'b0;
    always @(posedge clk) begin
        if (armed) begin
            if (free[G1-1:0]       != chain[G1-1:0])       diff_lo  <= 1'b1;
            if (free[G2-1:G1]      != chain[G2-1:G1])      diff_mid <= 1'b1;
            if (free[G3-1:G2]      != chain[G3-1:G2])      diff_hi  <= 1'b1;
            if (free[WIDTH-1:G3]   != chain[WIDTH-1:G3])   diff_top <= 1'b1;
        end
    end

    // Not a constant on a pad: a register that is only ever assigned
    // zero. `docs/fpga-xray.md` records that a pad tied to a constant
    // comes out low on this family, so a constant here would make the
    // control indistinguishable from the defect it is meant to exclude.
    reg zero_reg = 1'b0;
    always @(posedge clk) zero_reg <= 1'b0;

    assign led_free     = free[WIDTH-1];
    assign led_chain    = chain[WIDTH-1];
    assign led_diff_lo  = diff_lo;
    assign led_diff_mid = diff_mid;
    assign led_diff_hi  = diff_hi;
    assign led_diff_top = diff_top;
    assign led_zero     = zero_reg;
endmodule
