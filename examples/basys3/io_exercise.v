// Every input and output on a Digilent Basys 3, exercised at once, so
// that the board's map can be established before anything subtler is
// attempted.
//
// Sixteen switches, fifteen of the sixteen LEDs, five pushbuttons, four
// seven-segment digits with their decimal point. What it is for is to
// turn each of those from a transcription into a measurement: this
// repository's Basys 3 pin list came from Digilent's published master
// constraints and had never been checked, and the first thing checked in
// it — which end of the row `sw[0]` is at — turned out to be **wrong**
// (`examples/soc/board/basys3.rcf` said left, the board says right).
//
// ===================================================================
// WHAT IT DOES
// ===================================================================
//
// With no button held:
//
//   - **Each LED mirrors the switch below it.** `led[n] = sw[n]`, so
//     flipping one switch lights one LED and the two orderings are read
//     off together. This is the whole point of the design; everything
//     else is a bonus on the same bitstream.
//   - **The four digits show all sixteen switches as hexadecimal**, four
//     bits per digit, `sw[3:0]` on digit 0.
//   - **The decimal point blinks**, about three times every two seconds.
//     That is the liveness signal: a dark, still point means the design
//     is not running and nothing else on the glass means anything.
//
// Each button overrides the display while it is held. They are tests a
// person can ask for one at a time:
//
//   BTNC  every segment of every digit, and the point, lit at once.
//         Anything that stays dark is a segment that is not reaching the
//         glass. The digits are all driven together here rather than
//         multiplexed, so they are dimmer than usual and that is
//         expected.
//   BTNU  the digits show 0, 1, 2, 3 — digit 0 showing `0`. **This is
//         how to find which end of the display `an[0]` is at**, which
//         nothing in this repository knows. Read left to right: "0123"
//         means `an[0]` is the left-hand digit, "3210" means it is the
//         right-hand one.
//   BTNL  one digit only, `an[0]`, showing `8`. The same question asked
//         a second way, and at full brightness: whichever digit lights
//         is `an[0]`.
//   BTNR  one digit only, `an[3]`, showing `8`. The other end of the
//         same answer, so that BTNL and BTNR together cannot both be
//         misread.
//   BTND  all fifteen LEDs lit, ignoring the switches. Any LED that
//         stays dark is one that is not reaching the board — LD6 is
//         expected to be the only one, and it is not even in this
//         design.
//
// ===================================================================
// WHAT IS DELIBERATELY NOT IN IT
// ===================================================================
//
// **No adder.** `docs/fpga-xray.md` records a 26-bit `count + 1`
// measured computing the wrong value on this part, so every counter here
// is `blink.v`'s carry-free idiom — `count <= count ^ toggle` with
// `toggle[i] = &count[i-1:0]` — which maps onto lookup tables and no
// `CARRY4` at all. A design meant to establish the board's wiring must
// not rest on a part of the flow that is known broken, or a dark LED has
// two possible explanations instead of one.
//
// **No pad driven by a constant.** The same document records a pad tied
// to `1'b1` coming out low, because `techcells::drive_constant_data`
// does not cover an output buffer's input. Every output here is a
// multiplexer or a register output, so none of them depends on that.
//
// **LD6 is absent.** Ball U14 is the one LED of the sixteen in a
// `LIOB33_SING` tile, which this flow's IO tables do not describe;
// `blink_carry.rcf` and `bram_rom.v` already say so. Fifteen LEDs, and
// the gap between LD5 and LD7 is that tile and not a mistake.
//
// Segments and anodes are both active low — the display is common anode,
// switched by PNP transistors — which is Digilent's reference manual and
// still not something measured here. If the whole glass reads inverted,
// that is the thing to suspect, and BTNC is the test: it should light
// everything.

module io_exercise (
    input  wire        clk,
    input  wire [15:0] sw,
    input  wire        btn_c,
    input  wire        btn_u,
    input  wire        btn_d,
    input  wire        btn_l,
    input  wire        btn_r,

    // Fifteen LEDs: LD0 to LD5, then LD7 to LD15. See above for the gap.
    output wire [14:0] led,

    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    // ---- A carry-free counter, for the display multiplexer and the
    // ---- liveness blink. No `+` anywhere in this file.
    reg [25:0] count = 26'd0;
    wire [25:0] toggle;
    assign toggle[0] = 1'b1;
    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : lanes
            assign toggle[i] = &count[i-1:0];
        end
    endgenerate
    always @(posedge clk) count <= count ^ toggle;

    // Each digit held for 2^16 cycles: 655 microseconds, so 381 Hz
    // around all four — above the eye, below the point where the anode
    // transistors would smear one digit into the next.
    wire [1:0] digit = count[17:16];
    wire       blink = count[25];

    // ---- The buttons, each through two flip-flops. ----
    //
    // Not because a pushbutton needs to be synchronised to be read by a
    // person — it does not — but because an asynchronous input feeding
    // the multiplexer's select would be a genuine race, and this design
    // exists to produce unambiguous readings.
    reg [1:0] sc = 2'b00, su = 2'b00, sd = 2'b00, sl = 2'b00, sr = 2'b00;
    always @(posedge clk) begin
        sc <= {sc[0], btn_c};
        su <= {su[0], btn_u};
        sd <= {sd[0], btn_d};
        sl <= {sl[0], btn_l};
        sr <= {sr[0], btn_r};
    end
    wire hold_c = sc[1], hold_u = su[1], hold_d = sd[1],
         hold_l = sl[1], hold_r = sr[1];

    // ---- The LEDs: the switches, or all of them with BTND. ----
    //
    // `sw[6]` has no LED of its own, so the fifteen outputs skip it:
    // led[5] is LD5 and led[6] is LD7.
    assign led = hold_d ? 15'h7FFF
                        : {sw[15:7], sw[5:0]};

    // ---- What the digits show. ----
    wire [3:0] from_switches =
        (digit == 2'd0) ? sw[3:0]   :
        (digit == 2'd1) ? sw[7:4]   :
        (digit == 2'd2) ? sw[11:8]  :
                          sw[15:12];

    // BTNU: the digit's own index, which is what reveals the order.
    wire [3:0] from_index = {2'b00, digit};

    wire [3:0] nibble = hold_u ? from_index
                      : (hold_l || hold_r) ? 4'h8
                      : from_switches;

    // Active low, segments a..g in that order.
    // Active low, and **the bit order is `gfedcba`**: the leftmost bit of
    // a `7'b...` literal is the most significant, which is `seg[6]`, which
    // is segment g. The first version of this table was written as though
    // the leftmost bit were segment a, so every pattern was bit-reversed —
    // a `0` came out as a zero with no top bar and a lit middle bar.
    //
    // Nothing in simulation can catch that: the patterns are arbitrary
    // constants and a reversed font is as self-consistent as a correct
    // one. Only a person looking at the glass can, and one did.
    //
    // Checked against the standard active-low hex font: C0 F9 A4 B0 99 92
    // 82 F8 80 90 88 83 C6 A1 86 8E.
    function [6:0] segments;
        input [3:0] value;
        case (value)
            4'h0: segments = 7'b1000000;
            4'h1: segments = 7'b1111001;
            4'h2: segments = 7'b0100100;
            4'h3: segments = 7'b0110000;
            4'h4: segments = 7'b0011001;
            4'h5: segments = 7'b0010010;
            4'h6: segments = 7'b0000010;
            4'h7: segments = 7'b1111000;
            4'h8: segments = 7'b0000000;
            4'h9: segments = 7'b0010000;
            4'hA: segments = 7'b0001000;
            4'hB: segments = 7'b0000011;
            4'hC: segments = 7'b1000110;
            4'hD: segments = 7'b0100001;
            4'hE: segments = 7'b0000110;
            default: segments = 7'b0001110;  // F
        endcase
    endfunction

    // BTNC lights everything; otherwise the decode above.
    assign seg = hold_c ? 7'b0000000 : segments(nibble);

    // The point: lit for BTNC, otherwise the liveness blink.
    assign dp = hold_c ? 1'b0 : ~blink;

    // Which digits are enabled, active low. BTNC drives all four at
    // once, BTNL and BTNR one each, and otherwise one at a time.
    wire [3:0] one_hot = 4'd1 << digit;
    assign an = hold_c ? 4'b0000
              : hold_l ? 4'b1110
              : hold_r ? 4'b0111
              : ~one_hot;
endmodule
