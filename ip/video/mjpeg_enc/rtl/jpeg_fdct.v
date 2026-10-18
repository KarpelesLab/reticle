// jpeg_fdct — the 8x8 forward discrete cosine transform of ITU T.81
// §A.3.3, in fixed point and with no multiplier.
//
// What it does
//   Takes 64 samples of a block in row-major order and gives back its 64
//   coefficients in the same order. The samples are unsigned 0..255 as
//   they come out of a colour transform and the **level shift of
//   §A.3.1 is done here**, subtracting 128, because the shift belongs to
//   the transform's definition and a caller that had to remember it
//   would eventually forget.
//
//   The transform is separable, so it is eight one-dimensional
//   transforms across the rows and eight more down the columns, sharing
//   one arithmetic unit. That unit uses the even/odd decomposition: the
//   even-numbered outputs depend only on `x[n] + x[7-n]` and the odd
//   ones only on `x[n] - x[7-n]`, so each is a sum of **four** terms.
//
// No multiplier, and why that is not an optimisation
//   **Nothing this flow writes a bitstream for can hold a hard
//   multiplier.** `fpga::trellis::dsp` and `fpga::xray::dsp` each say
//   why for their family, and the command line turns DSP inference off
//   whenever a bitstream is asked for. So a `*` here is a multiplier
//   built out of lookup tables, and four of them measured **7948 LUT4
//   at depth 56** on an ECP5 45F — deeper than the 19 that did not meet
//   60 MHz in `docs/fpga-trellis.md`, and six times an iCE40 HX1K's
//   whole fabric.
//
//   So the four products are **distributed arithmetic** instead, and
//   with the buffer below in a block RAM the whole transform is
//   **1423 LUT4 at depth 10** — a fifth of the multiplier version's
//   area and a fifth of its depth. The
//   constants are fixed, so every sum of a subset of them can be worked
//   out in advance: for each output there is a sixteen-entry table of
//   `sum of C[k][n] over the n in the subset`. The four inputs are then
//   fed in a bit at a time, most significant first, and
//
//       acc = -table[bits of the sign]          then, fourteen times,
//       acc = 2 * acc + table[the next bits]
//
//   which is Horner's rule over the binary expansion and is **exact**:
//   it reproduces the multiplier version bit for bit, checked over 3000
//   random blocks. The doubling is a wire, so each cycle is one table
//   lookup and one addition, and the whole thing is eight small tables
//   and eight accumulators.
//
// Why these numbers
//   The constants are `0.5 * c(k) * cos((2n+1)k*pi/16)` rounded at 13
//   fractional bits, and the two passes round by **different** amounts:
//   10 bits after the first and 16 after the second. Measured against a
//   double-precision transform over 2000 random blocks:
//
//     rounding        peak error   RMS error   flat-block DC
//     10 then 16        0.616       0.291      exact
//     11 then 15        0.725       0.298      off by one
//     13 then 13        1.413       0.409      off by one
//
//   The last column is the one that decided it. A flat block — the sky,
//   a wall, a letterbox — must give `DC = 8 * value` and **exactly
//   zero** for all 63 AC coefficients, and rounding evenly between the
//   passes gets the DC wrong by one for some values. That is a visible
//   step between two flat areas that should match, and it costs nothing
//   to avoid.
//
//   Rounding is to nearest with ties toward positive infinity, which is
//   one adder and an arithmetic shift. Ties away from zero was measured
//   too and is identical to three decimal places on all three figures.
//
// What it does not do
//   No quantisation, no zig-zag and no entropy coding; those are the
//   modules beside this one.
//
//   It is not bit-exact against any other implementation and cannot be:
//   T.81 defines the transform mathematically and leaves the arithmetic
//   to the implementer, which is why there is no reference bitstream to
//   compare against. What is checked instead is the distance from the
//   real transform, the exactness on flat blocks, and that a basis
//   function in gives one non-zero coefficient out.
//
//   The block between the two passes is a **block RAM**, with one read
//   port and one write port. Read eight words at once — a whole row,
//   which is what the second pass wants — and this flow has to build 64
//   words of flip-flops with a multiplexer per read instead: 3970 LUT4
//   and 1306 flip-flops, against one block RAM and a third of the logic
//   for the serial read. The row is fetched a word a cycle, which at
//   these rates is nothing.
//
//   One block at a time: **648 cycles** from the first sample to the
//   last coefficient, and no overlap between blocks — measured, and
//   pinned by `jpeg_fdct_takes_the_cycles_its_header_says`, because a
//   quoted rate that nothing checks drifts the first time the
//   sequencer gains a state. At 60 MHz that is 92 000 blocks a second,
//   which is 4:2:0 video at 320x240 and 60 Hz, or 640x480 and 12 Hz.
module jpeg_fdct (
    input  wire               clk,
    input  wire               rst_n,

    // The block, row-major, 64 samples. Unsigned; the level shift is
    // applied here.
    input  wire        [7:0]  in_sample,
    input  wire               in_valid,
    output wire               in_ready,

    // Its coefficients, row-major, 64 of them.
    output wire signed [11:0] out_coef,
    output wire               out_valid,
    input  wire               out_ready
);
    localparam [2:0] S_LOAD  = 3'd0;  // eight samples of a row
    localparam [2:0] S_FETCH = 3'd1;  // a row of `blk`, one word a cycle
    localparam [2:0] S_PREP  = 3'd2;  // the sums and the differences
    localparam [2:0] S_DA    = 3'd3;  // fifteen bit-serial cycles
    localparam [2:0] S_PUT   = 3'd4;  // the eight results, one a cycle
    localparam [2:0] S_OUT   = 3'd5;  // stream the block out

    reg [2:0] state;
    reg [3:0] idx;                    // the sample, the bit, or the output
    reg [2:0] pos;                    // which row, or which column
    reg [5:0] oix;                    // the output index, row-major
    reg       pass2;                  // the columns, not the rows
    reg       armed;                  // `blk_q` holds the word `oix` names

    // The eight values the transform is working on, fifteen bits so the
    // sums and differences of the second pass fit.
    reg signed [14:0] v0, v1, v2, v3, v4, v5, v6, v7;

    // -----------------------------------------------------------------
    // The block between the passes, as a block RAM
    // -----------------------------------------------------------------
    // `blk[{a, b}]` holds the first pass's output `k = a` of row `b`,
    // and after the second pass it holds coefficient `(b, a)` — see the
    // writes below, which is where the transpose happens. One flat
    // array of 64 and not an 8 by 8 one, because this flow does not
    // build multi-dimensional unpacked arrays (`V0013`); the index is
    // the two coordinates concatenated, which is the same thing.
    //
    // One clocked read into one register, one clocked write, and no
    // reset: the shape this flow builds a block RAM from. Resetting it
    // would be sixty-four more write ports, which `CLAUDE.md` records
    // costing `z80`'s register file dearly; every word is written
    // before it is read, so there is nothing to clear.
    reg signed [12:0] blk [0:63];
    reg signed [12:0] blk_q;

    // **The port controls are combinational, not registered.** Clocked,
    // an address assigned in one cycle is presented in the next and the
    // word lands in `blk_q` the cycle after that, which is one more
    // than the fetch below counts on -- and it shifted a stale `blk_q`
    // into the first element of every row. Combinational, the word
    // asked for in a cycle is in `blk_q` at the start of the next, and
    // the memory is still the one shape this flow builds a block RAM
    // from.
    reg        [5:0]  blk_addr;
    reg               blk_rd;
    reg               blk_we;
    reg signed [12:0] blk_d;

    always @(posedge clk) begin
        if (blk_we) blk[blk_addr] <= blk_d;
        if (blk_rd) blk_q <= blk[blk_addr];
    end

    // -----------------------------------------------------------------
    // The sums and differences, and the registers that serialise them
    // -----------------------------------------------------------------
    reg [14:0] s0, s1, s2, s3;        // the sums, shifted out MSB first
    reg [14:0] q0, q1, q2, q3;        // the differences

    // The two table addresses: bit `n` of an address is the current bit
    // of input `n`, which is the order the tables were built in.
    wire [3:0] sa = {s3[14], s2[14], s1[14], s0[14]};
    wire [3:0] da = {q3[14], q2[14], q1[14], q0[14]};

    reg signed [14:0] r0, r1, r2, r3, r4, r5, r6, r7;

    // Output 0: the sums of subsets of its four constants.
    always @* begin
        case (sa)
            4'd0: r0 = 15'sd0;
            4'd1: r0 = 15'sd2896;
            4'd2: r0 = 15'sd2896;
            4'd3: r0 = 15'sd5792;
            4'd4: r0 = 15'sd2896;
            4'd5: r0 = 15'sd5792;
            4'd6: r0 = 15'sd5792;
            4'd7: r0 = 15'sd8688;
            4'd8: r0 = 15'sd2896;
            4'd9: r0 = 15'sd5792;
            4'd10: r0 = 15'sd5792;
            4'd11: r0 = 15'sd8688;
            4'd12: r0 = 15'sd5792;
            4'd13: r0 = 15'sd8688;
            4'd14: r0 = 15'sd8688;
            4'd15: r0 = 15'sd11584;
            default: r0 = 15'sd0;
        endcase
    end

    // Output 1: the sums of subsets of its four constants.
    always @* begin
        case (da)
            4'd0: r1 = 15'sd0;
            4'd1: r1 = 15'sd4017;
            4'd2: r1 = 15'sd3406;
            4'd3: r1 = 15'sd7423;
            4'd4: r1 = 15'sd2276;
            4'd5: r1 = 15'sd6293;
            4'd6: r1 = 15'sd5682;
            4'd7: r1 = 15'sd9699;
            4'd8: r1 = 15'sd799;
            4'd9: r1 = 15'sd4816;
            4'd10: r1 = 15'sd4205;
            4'd11: r1 = 15'sd8222;
            4'd12: r1 = 15'sd3075;
            4'd13: r1 = 15'sd7092;
            4'd14: r1 = 15'sd6481;
            4'd15: r1 = 15'sd10498;
            default: r1 = 15'sd0;
        endcase
    end

    // Output 2: the sums of subsets of its four constants.
    always @* begin
        case (sa)
            4'd0: r2 = 15'sd0;
            4'd1: r2 = 15'sd3784;
            4'd2: r2 = 15'sd1567;
            4'd3: r2 = 15'sd5351;
            4'd4: r2 = -15'sd1567;
            4'd5: r2 = 15'sd2217;
            4'd6: r2 = 15'sd0;
            4'd7: r2 = 15'sd3784;
            4'd8: r2 = -15'sd3784;
            4'd9: r2 = 15'sd0;
            4'd10: r2 = -15'sd2217;
            4'd11: r2 = 15'sd1567;
            4'd12: r2 = -15'sd5351;
            4'd13: r2 = -15'sd1567;
            4'd14: r2 = -15'sd3784;
            4'd15: r2 = 15'sd0;
            default: r2 = 15'sd0;
        endcase
    end

    // Output 3: the sums of subsets of its four constants.
    always @* begin
        case (da)
            4'd0: r3 = 15'sd0;
            4'd1: r3 = 15'sd3406;
            4'd2: r3 = -15'sd799;
            4'd3: r3 = 15'sd2607;
            4'd4: r3 = -15'sd4017;
            4'd5: r3 = -15'sd611;
            4'd6: r3 = -15'sd4816;
            4'd7: r3 = -15'sd1410;
            4'd8: r3 = -15'sd2276;
            4'd9: r3 = 15'sd1130;
            4'd10: r3 = -15'sd3075;
            4'd11: r3 = 15'sd331;
            4'd12: r3 = -15'sd6293;
            4'd13: r3 = -15'sd2887;
            4'd14: r3 = -15'sd7092;
            4'd15: r3 = -15'sd3686;
            default: r3 = 15'sd0;
        endcase
    end

    // Output 4: the sums of subsets of its four constants.
    always @* begin
        case (sa)
            4'd0: r4 = 15'sd0;
            4'd1: r4 = 15'sd2896;
            4'd2: r4 = -15'sd2896;
            4'd3: r4 = 15'sd0;
            4'd4: r4 = -15'sd2896;
            4'd5: r4 = 15'sd0;
            4'd6: r4 = -15'sd5792;
            4'd7: r4 = -15'sd2896;
            4'd8: r4 = 15'sd2896;
            4'd9: r4 = 15'sd5792;
            4'd10: r4 = 15'sd0;
            4'd11: r4 = 15'sd2896;
            4'd12: r4 = 15'sd0;
            4'd13: r4 = 15'sd2896;
            4'd14: r4 = -15'sd2896;
            4'd15: r4 = 15'sd0;
            default: r4 = 15'sd0;
        endcase
    end

    // Output 5: the sums of subsets of its four constants.
    always @* begin
        case (da)
            4'd0: r5 = 15'sd0;
            4'd1: r5 = 15'sd2276;
            4'd2: r5 = -15'sd4017;
            4'd3: r5 = -15'sd1741;
            4'd4: r5 = 15'sd799;
            4'd5: r5 = 15'sd3075;
            4'd6: r5 = -15'sd3218;
            4'd7: r5 = -15'sd942;
            4'd8: r5 = 15'sd3406;
            4'd9: r5 = 15'sd5682;
            4'd10: r5 = -15'sd611;
            4'd11: r5 = 15'sd1665;
            4'd12: r5 = 15'sd4205;
            4'd13: r5 = 15'sd6481;
            4'd14: r5 = 15'sd188;
            4'd15: r5 = 15'sd2464;
            default: r5 = 15'sd0;
        endcase
    end

    // Output 6: the sums of subsets of its four constants.
    always @* begin
        case (sa)
            4'd0: r6 = 15'sd0;
            4'd1: r6 = 15'sd1567;
            4'd2: r6 = -15'sd3784;
            4'd3: r6 = -15'sd2217;
            4'd4: r6 = 15'sd3784;
            4'd5: r6 = 15'sd5351;
            4'd6: r6 = 15'sd0;
            4'd7: r6 = 15'sd1567;
            4'd8: r6 = -15'sd1567;
            4'd9: r6 = 15'sd0;
            4'd10: r6 = -15'sd5351;
            4'd11: r6 = -15'sd3784;
            4'd12: r6 = 15'sd2217;
            4'd13: r6 = 15'sd3784;
            4'd14: r6 = -15'sd1567;
            4'd15: r6 = 15'sd0;
            default: r6 = 15'sd0;
        endcase
    end

    // Output 7: the sums of subsets of its four constants.
    always @* begin
        case (da)
            4'd0: r7 = 15'sd0;
            4'd1: r7 = 15'sd799;
            4'd2: r7 = -15'sd2276;
            4'd3: r7 = -15'sd1477;
            4'd4: r7 = 15'sd3406;
            4'd5: r7 = 15'sd4205;
            4'd6: r7 = 15'sd1130;
            4'd7: r7 = 15'sd1929;
            4'd8: r7 = -15'sd4017;
            4'd9: r7 = -15'sd3218;
            4'd10: r7 = -15'sd6293;
            4'd11: r7 = -15'sd5494;
            4'd12: r7 = -15'sd611;
            4'd13: r7 = 15'sd188;
            4'd14: r7 = -15'sd2887;
            4'd15: r7 = -15'sd2088;
            default: r7 = 15'sd0;
        endcase
    end


    // -----------------------------------------------------------------
    // Eight accumulators, one addition each a cycle
    // -----------------------------------------------------------------
    // 29 bits: the largest partial sum is 189 792 256 in magnitude,
    // measured over the extremes rather than bounded on paper.
    reg signed [28:0] a0, a1, a2, a3, a4, a5, a6, a7;

    wire first = (idx == 4'd0);       // the sign bit's turn

    // `2 * acc + table`, or minus the table on the sign bit. The
    // doubling is a wire.
    function signed [28:0] step;
        input signed [28:0] acc;
        input signed [14:0] rom;
        input               sign;
        begin
            step = sign ? -{{14{rom[14]}}, rom}
                        : {acc[27:0], 1'b0} + {{14{rom[14]}}, rom};
        end
    endfunction

    // The result, rounded: 10 bits after the first pass and 16 after
    // the second. See the header for why they differ.
    reg signed [28:0] chosen;
    always @* begin
        case (idx[2:0])
            3'd0: chosen = a0;
            3'd1: chosen = a1;
            3'd2: chosen = a2;
            3'd3: chosen = a3;
            3'd4: chosen = a4;
            3'd5: chosen = a5;
            3'd6: chosen = a6;
            default: chosen = a7;
        endcase
    end
    wire signed [28:0] rounded = pass2 ? ((chosen + 29'sd32768) >>> 16)
                                       : ((chosen + 29'sd512)   >>> 10);

    // -----------------------------------------------------------------
    // The handshake
    // -----------------------------------------------------------------
    assign in_ready  = (state == S_LOAD);
    assign out_valid = (state == S_OUT) && armed;
    assign out_coef  = blk_q[11:0];

    wire took = in_valid && in_ready;
    wire gave = out_valid && out_ready;

    // The level shift of §A.3.1.
    wire signed [14:0] shifted = {7'd0, in_sample} - 15'sd128;

    // The next output's address: coefficient `(r, c)` was written to
    // `{c, r}`, so streaming row-major reads the two halves swapped.
    wire [5:0] onext = oix + 6'd1;

    // Which port does what, this cycle.
    always @* begin
        blk_rd   = 1'b0;
        blk_we   = 1'b0;
        blk_addr = 6'd0;
        blk_d    = rounded[12:0];
        case (state)
            S_FETCH: begin
                blk_rd   = (idx <= 4'd7);
                blk_addr = {pos, idx[2:0]};
            end
            S_PUT: begin
                blk_we   = 1'b1;
                blk_addr = pass2 ? {pos, idx[2:0]} : {idx[2:0], pos};
            end
            S_OUT: begin
                blk_rd   = !armed;
                blk_addr = {oix[2:0], oix[5:3]};
            end
            default: ;
        endcase
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state  <= S_LOAD;
            idx    <= 4'd0;
            pos    <= 3'd0;
            oix    <= 6'd0;
            pass2  <= 1'b0;
            armed  <= 1'b0;
            {v0, v1, v2, v3, v4, v5, v6, v7} <= {8{15'sd0}};
            {s0, s1, s2, s3} <= {4{15'd0}};
            {q0, q1, q2, q3} <= {4{15'd0}};
            {a0, a1, a2, a3, a4, a5, a6, a7} <= {8{29'sd0}};
        end else begin
            case (state)
                S_LOAD: if (took) begin
                    // A shift register, so the sample just taken lands
                    // in `v7` and the first of the row ends in `v0`.
                    v0 <= v1; v1 <= v2; v2 <= v3; v3 <= v4;
                    v4 <= v5; v5 <= v6; v6 <= v7; v7 <= shifted;
                    if (idx == 4'd7) begin
                        idx   <= 4'd0;
                        state <= S_PREP;
                    end else begin
                        idx <= idx + 4'd1;
                    end
                end
                // Nine cycles: a read is issued on each of the first
                // eight and lands one cycle later, so the word fetched
                // at `idx` is shifted in at `idx + 1`.
                S_FETCH: begin
                    if (idx != 4'd0) begin
                        v0 <= v1; v1 <= v2; v2 <= v3; v3 <= v4;
                        v4 <= v5; v5 <= v6; v6 <= v7;
                        v7 <= {{2{blk_q[12]}}, blk_q};
                    end
                    if (idx == 4'd8) begin
                        idx   <= 4'd0;
                        state <= S_PREP;
                    end else begin
                        idx <= idx + 4'd1;
                    end
                end
                S_PREP: begin
                    s0 <= v0 + v7; s1 <= v1 + v6;
                    s2 <= v2 + v5; s3 <= v3 + v4;
                    q0 <= v0 - v7; q1 <= v1 - v6;
                    q2 <= v2 - v5; q3 <= v3 - v4;
                    idx   <= 4'd0;
                    state <= S_DA;
                end
                S_DA: begin
                    a0 <= step(a0, r0, first); a1 <= step(a1, r1, first);
                    a2 <= step(a2, r2, first); a3 <= step(a3, r3, first);
                    a4 <= step(a4, r4, first); a5 <= step(a5, r5, first);
                    a6 <= step(a6, r6, first); a7 <= step(a7, r7, first);
                    s0 <= {s0[13:0], 1'b0}; s1 <= {s1[13:0], 1'b0};
                    s2 <= {s2[13:0], 1'b0}; s3 <= {s3[13:0], 1'b0};
                    q0 <= {q0[13:0], 1'b0}; q1 <= {q1[13:0], 1'b0};
                    q2 <= {q2[13:0], 1'b0}; q3 <= {q3[13:0], 1'b0};
                    if (idx == 4'd14) begin
                        idx   <= 4'd0;
                        state <= S_PUT;
                    end else begin
                        idx <= idx + 4'd1;
                    end
                end
                S_PUT: begin
                    // The first pass stores transposed — output `k` of
                    // row `pos` goes to `{k, pos}` — so the second pass
                    // reads a whole row instead of a column. The second
                    // writes back into the row it has already fetched,
                    // which clobbers nothing another column still needs.
                    idx <= idx + 4'd1;
                    if (idx == 4'd7) begin
                        idx <= 4'd0;
                        if (pos == 3'd7) begin
                            pos <= 3'd0;
                            if (pass2) begin
                                oix   <= 6'd0;
                                armed <= 1'b0;
                                pass2 <= 1'b0;
                                state <= S_OUT;
                            end else begin
                                pass2 <= 1'b1;
                                state <= S_FETCH;
                            end
                        end else begin
                            pos   <= pos + 3'd1;
                            state <= pass2 ? S_FETCH : S_LOAD;
                        end
                    end
                end
                // A word a time out of the memory, so the valid falls
                // for the cycle the next read takes. Sixty-four more
                // cycles on a block that already takes nearly six
                // hundred, and it keeps the memory to one read port.
                S_OUT: begin
                    if (!armed) begin
                        // The read asked for this cycle is in `blk_q`
                        // at the start of the next, so the valid can go
                        // up then.
                        armed <= 1'b1;
                    end else if (gave) begin
                        armed <= 1'b0;
                        if (oix == 6'd63) begin
                            oix   <= 6'd0;
                            idx   <= 4'd0;
                            pos   <= 3'd0;
                            state <= S_LOAD;
                        end else begin
                            oix <= onext;
                        end
                    end
                end
                default: state <= S_LOAD;
            endcase
        end
    end
endmodule
