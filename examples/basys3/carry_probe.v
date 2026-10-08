// Two counters that must agree, one using a `CARRY4` chain and one not,
// on a Digilent Basys 3.
//
// NOTHING ABOUT THE CARRY CHAIN IS ESTABLISHED BY EITHER READING TAKEN
// ON 8 OCTOBER 2026, and the retracted text is kept below because the
// way it misled is the useful part. Both readings were taken through a
// routing-graph defect — a `tileconn` join that landed on the wrong
// tile type and welded two interconnect rows together — that corrupted
// the carry-free counter this design compares against.
// `docs/fpga-xray.md` accounts for every LED of the second reading
// without the carry chain being at fault at all. Use
// `carry_probe_trusted.rcf`, and read the doc first.
//
// RETRACTED: "a person at the board read LD4 **lit**, which is the latch
// that closes the first cycle the two counters differ. So `count + 1` at
// 26 bits does not compute plus one on this part." LD4 is ball W18,
// which was later found not to follow its net; and in the re-run on
// trusted balls the latch closed honestly, because `plain[2]` lost two
// hops to the welded join and the two counters really did differ.
//
// Why this design and not `blink_carry.v`: that one asks a person to
// judge whether each LED blinks at half the rate of the LED to its
// right, over periods from 24 Hz to 43 seconds. This asks nothing of
// anybody. Both counters start at zero and both add one per cycle, so
// they are equal on *every* cycle, and LD4 latches the first cycle they
// are not. The answer is one LED, lit or dark, and no stopwatch.
//
// `carry_probe_tb.v` proves the design itself is right: 5000 cycles with
// the two counters agreeing exactly, so a disagreement on the part is
// the part's answer and not this file's. That order matters — the design
// was simulated first, precisely so that the hardware result could not
// be blamed on the RTL.
//
// WHAT IT DOES NOT ESTABLISH: *which* carry is wrong. A chain whose
// `COUT` never reaches the next cell's `CIN` and a chain whose first
// `CYINIT` is zero both show up here as one lit LED. The bits above the
// first four would tell the difference and this design does not report
// them; it was built to answer "is there a fault" without a person
// having to interpret anything, and that is all it answers.
//
// A SECOND FAULT, whose code half is real whatever the board says: LD0
// is driven by the constant `1'b1` and read **dark** (on `carry_probe.rcf`
// beside LD1 driven by `1'b0`, which also read dark; on
// `carry_probe_trusted.rcf` LD14 was the constant one and read dark
// while the constant zero on LD11 correctly did too). `techcells::drive_constant_data` builds a real driver
// for a constant that reaches a flip-flop's data pin, a distributed
// RAM's inputs or a carry cell's operands, and **not** for one that
// reaches an output buffer, so the pad's input is left unrouted. On this
// family that reads low; on an ECP5 an unrouted input reads as a one,
// which is the same defect with the opposite symptom and is the one
// `CLAUDE.md` records eight rounds lost to.
//
//   LD0  a constant one          LIT   (this is the one under suspicion)
//   LD1  a constant zero         DARK  (the control for LD0)
//   LD2  carry-free counter b25  BLINKS slowly, about 1.5 Hz
//   LD3  carry-chain counter b25 BLINKS slowly, in step with LD2
//   LD4  "they ever differed"    DARK  <- the whole experiment
//   LD5  "the carry counter ever left zero"  LIT
//   LD7  carry-free counter b22  FLICKERS, about 12 Hz
//
// LD4 dark and LD5 lit together mean the carry chain counts and agrees
// with logic that uses no carry at all. LD4 lit means it does not, and
// `blink_carry.v` is wrong on silicon. LD5 dark with LD2 blinking means
// the carry counter never moved.
module carry_probe (
    input  wire clk,
    output wire led_one,
    output wire led_zero,
    output wire led_plain,
    output wire led_carry,
    output wire led_differed,
    output wire led_moved,
    output wire led_alive
);
    // ---- The carry-free counter: `blink.v`'s idiom, no CARRY4. ----
    reg [25:0] plain = 26'd0;
    wire [25:0] toggle;
    assign toggle[0] = 1'b1;
    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : lanes
            assign toggle[i] = &plain[i-1:0];
        end
    endgenerate
    always @(posedge clk) plain <= plain ^ toggle;

    // ---- The carry-chain counter: the obvious spelling. ----
    reg [25:0] chain = 26'd0;
    always @(posedge clk) chain <= chain + 26'd1;

    // ---- The comparison, latched. ----
    reg differed = 1'b0;
    reg moved    = 1'b0;
    always @(posedge clk) begin
        if (plain != chain) differed <= 1'b1;
        if (chain != 26'd0) moved    <= 1'b1;
    end

    assign led_one      = 1'b1;
    assign led_zero     = 1'b0;
    assign led_plain    = plain[25];
    assign led_carry    = chain[25];
    assign led_differed = differed;
    assign led_moved    = moved;
    assign led_alive    = plain[22];
endmodule
