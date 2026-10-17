// adpcm_ima_enc — IMA/DVI ADPCM: one 16-bit sample in, one 4-bit
// nibble out, 4:1 and fixed rate.
//
// What it does
//   The encoder of the scheme WAV calls format 0x11 and AVI carries
//   beside MJPEG. It keeps two pieces of state — a predictor, which is
//   the sample it believes it has reconstructed so far, and a step
//   index into `adpcm_step`'s table — and for each sample emits the
//   four bits that move the predictor closest to it: a sign and three
//   magnitude bits worth step, step/2 and step/4, over a floor of
//   step/8.
//
//   **The predictor this block carries is the one a decoder will
//   build.** Encoder and decoder run the same reconstruction, so
//   `predictor` here is exactly what any conforming decoder produces
//   for the nibbles already emitted. That is what makes the round trip
//   checkable rather than approximately checkable, and it is why
//   `predictor` and `step_index` are brought out: they are also the two
//   fields a WAV block header carries.
//
// Five cycles a sample, and why not one
//   The arithmetic is a chain: the difference, its magnitude, three
//   compare-and-subtract steps, the reconstruction sum, the predictor
//   and its clamp. Written as one cycle of combinational logic that is
//   six ripple carries end to end, and it measured **54 logic levels
//   on an ECP5 and 72 on a four-input fabric** — which is deeper than
//   the design in `docs/fpga-trellis.md` that did not meet 60 MHz and
//   took four rounds to diagnose. As a sequencer with the table
//   registered it is **14 on both**, for 79 more lookup tables and
//   fifteen more flip-flops. The README has the four measurements.
//
//   So it is a sequencer instead, with **one arithmetic operation per
//   cycle**: a sample is taken in `S_IDLE` and the nibble is ready four
//   cycles later. Nothing is lost. Audio arrives at tens of kilohertz
//   and five cycles at even 1 MHz is forty times faster than 48 kHz
//   needs, so the rate was never the question — and a feedback codec
//   cannot be pipelined for back-to-back samples anyway, because the
//   next sample's difference needs this sample's predictor. Iterating
//   is the only shape that helps, and it makes the block smaller as
//   well as shallower.
//
//   Two tricks keep each state to one carry. **A comparison is the
//   subtraction's borrow**: `acc - w` is computed once and its borrow
//   says whether `acc` was the smaller, so the quantiser's three
//   compares cost nothing beyond the subtracts it already does. And
//   **the clamp to sixteen bits is a test on the top bits**, not a pair
//   of comparators: a 19-bit sum fits in `int16` exactly when its top
//   three bits equal its sixteenth.
//
//   `load` seeds the state, which is what the start of a WAV block
//   needs: each block carries its own predictor and step index, and a
//   decoder restarts from them. It touches the state only and not the
//   output slot, so it must be asserted between blocks, with no nibble
//   pending. Asserting it with `out_valid` high leaves a nibble that
//   belongs to the previous block in the slot, which is a caller error
//   this block cannot detect. It is ignored while a sample is in
//   flight, because a block boundary in the middle of one is not a
//   thing the format can express.
//
// Why this quantiser and not ffmpeg's
//   The nibble a decoder must turn back into a sample is specified; the
//   nibble an encoder chooses for a sample is not. This block
//   implements the subtractive quantiser of the IMA reference, and
//   ffmpeg's `adpcm_ima_wav` encoder reaches a different nibble on some
//   samples. Both are legal and `ip/audio/adpcm_ima/README.md` has the
//   measurement: they are within 0.15 dB of each other on signals near
//   full scale, and on a quiet one (plus or minus 50 counts) this block
//   is **3 dB better**, 15.4 against 12.4. What is *not* a matter of
//   choice is the decoder, and `adpcm_ima_dec` is checked sample for
//   sample against ffmpeg's.
//
// What it does not do
//   No block framing: no RIFF, no WAV header, no per-block headers and
//   no nibble packing. §"Writing a WAV block" in the README has the
//   layout a caller has to produce.
//
//   No stereo. One channel; a stereo stream wants two instances,
//   because the per-channel state is independent and WAV interleaves
//   the two channels' nibbles in four-byte groups.
//
//   No lookahead and no trellis, which is the other thing ffmpeg can do
//   (`-trellis 1`) and which does measurably better — 39.3 against 47.4
//   in sum of squares on a square wave. It needs a search over future
//   samples and a buffer to hold them, and this block has neither.
module adpcm_ima_enc (
    input  wire               clk,
    input  wire               rst_n,

    // Seed the state at the start of a block. See the header.
    input  wire               load,
    input  wire signed [15:0] load_pred,
    input  wire        [6:0]  load_index,

    input  wire signed [15:0] in_sample,
    input  wire               in_valid,
    output wire               in_ready,

    output wire        [3:0]  out_nibble,
    output wire               out_valid,
    input  wire               out_ready,

    // The reconstruction so far, and the step index: what a decoder
    // will have, and what a block header carries.
    output wire signed [15:0] predictor,
    output wire        [6:0]  step_index
);
    localparam [2:0] S_IDLE = 3'd0;
    localparam [2:0] S_Q2   = 3'd1;   // the step bit
    localparam [2:0] S_Q1   = 3'd2;   // the step/2 bit
    localparam [2:0] S_Q0   = 3'd3;   // the step/4 bit
    localparam [2:0] S_UPD  = 3'd4;   // the predictor and the index

    reg [2:0]         state;
    reg signed [15:0] pred_q;
    reg        [6:0]  index_q;
    reg        [16:0] acc_q;          // the magnitude, then what is left of it
    reg        [16:0] vp_q;           // the reconstruction, as it is built
    reg               sign_q;
    reg        [2:0]  delta_q;
    reg        [3:0]  nib_q;
    reg               ov_q;

    // **The table is registered, so it is never in series with an
    // adder.** `index_q` only moves in `S_UPD`, so the lookup can be
    // done on the value the index is *about* to take and latched beside
    // it: `step_q` and `index_q` always agree. Looked up combinationally
    // from `index_q` instead, the 89-entry multiplexer sat in front of
    // the subtract that uses its result and the block measured 20 logic
    // levels -- the same depth as the design in `docs/fpga-trellis.md`
    // that did not meet 60 MHz. Fifteen flip-flops buy that back.
    wire [6:0] index_nx = load                ? ((load_index > 7'd88) ? 7'd88
                                                                     : load_index)
                        : (state == S_UPD)    ? inext
                        :                       index_q;
    wire [14:0] step_d;
    adpcm_step u_step (.index(index_nx), .step(step_d));
    reg  [14:0] step_q;
    wire [14:0] step = step_q;

    wire [16:0] w1 = {2'b0, step};              // step
    wire [16:0] w2 = {3'b0, step[14:1]};        // step >> 1
    wire [16:0] w4 = {4'b0, step[14:2]};        // step >> 2
    wire [16:0] w8 = {5'b0, step[14:3]};        // step >> 3, the floor

    // -----------------------------------------------------------------
    // S_IDLE: the difference and its magnitude, in one carry
    // -----------------------------------------------------------------
    // Both directions are subtracted at once and the sign of one of
    // them picks between the results, so the absolute value costs one
    // chain and a multiplexer rather than a subtract followed by a
    // negate. Seventeen bits because the extremes are 32767 - (-32768)
    // and its negation, which span -65535 to 65535.
    wire signed [17:0] dpos = {{2{in_sample[15]}}, in_sample}
                            - {{2{pred_q[15]}}, pred_q};
    wire signed [17:0] dneg = {{2{pred_q[15]}}, pred_q}
                            - {{2{in_sample[15]}}, in_sample};
    wire               dsign = dpos[17];
    wire        [16:0] dmag  = dsign ? dneg[16:0] : dpos[16:0];

    // -----------------------------------------------------------------
    // The quantiser: one subtract a state, the borrow as the comparison
    // -----------------------------------------------------------------
    wire [16:0] wsel = (state == S_Q2) ? w1 : (state == S_Q1) ? w2 : w4;
    wire [17:0] sub  = {1'b0, acc_q} - {1'b0, wsel};
    wire        fits = !sub[17];                // acc_q >= wsel
    // The addend is formed whether or not the bit is set, so the sum is
    // not waiting on the subtract's borrow.
    wire [16:0] vp_next = (state == S_Q2) ? (w8 + w1) : (vp_q + wsel);

    // -----------------------------------------------------------------
    // S_UPD: the predictor, its clamp, and the step index
    // -----------------------------------------------------------------
    wire signed [18:0] pwide = sign_q
        ? ({{3{pred_q[15]}}, pred_q} - {2'b0, vp_q})
        : ({{3{pred_q[15]}}, pred_q} + {2'b0, vp_q});
    // A 19-bit two's-complement value fits in `int16` exactly when its
    // top three bits are copies of bit 15. That is a three-bit test and
    // not two 19-bit comparisons.
    wire               pfits = (pwide[18:16] == {3{pwide[15]}});
    wire signed [15:0] pnext = pfits ? pwide[15:0]
                             : (pwide[18] ? 16'sh8000 : 16'sh7FFF);

    // -1 below four, and 2, 4, 6, 8 at four to seven, which is
    // 2 * (delta - 3). The sum spans -1 to 96, so eight bits.
    wire signed [7:0] adj   = delta_q[2] ? (({5'd0, delta_q} - 8'sd3) <<< 1)
                                         : -8'sd1;
    wire signed [8:0] iwide = {2'b0, index_q} + {adj[7], adj};
    wire        [6:0] inext = iwide[8]          ? 7'd0
                            : (iwide > 9'sd88)  ? 7'd88
                            :                     iwide[6:0];

    // -----------------------------------------------------------------
    // The handshake
    // -----------------------------------------------------------------
    assign in_ready   = (state == S_IDLE) && (!ov_q || out_ready);
    assign out_nibble = nib_q;
    assign out_valid  = ov_q;
    assign predictor  = pred_q;
    assign step_index = index_q;

    wire take = in_valid && in_ready;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state   <= S_IDLE;
            pred_q  <= 16'sd0;
            index_q <= 7'd0;
            acc_q   <= 17'd0;
            vp_q    <= 17'd0;
            sign_q  <= 1'b0;
            delta_q <= 3'd0;
            nib_q   <= 4'd0;
            ov_q    <= 1'b0;
            step_q  <= 15'd7;            // the step at index 0
        end else begin
            step_q <= step_d;
            if (ov_q && out_ready && (state != S_UPD)) ov_q <= 1'b0;
            case (state)
                S_IDLE: begin
                    // `load` is honoured here and nowhere else: this is
                    // the only state in which no sample is in flight.
                    if (load) begin
                        pred_q  <= load_pred;
                        index_q <= index_nx;
                    end else if (take) begin
                        sign_q  <= dsign;
                        acc_q   <= dmag;
                        delta_q <= 3'd0;
                        state   <= S_Q2;
                    end
                end
                S_Q2: begin
                    if (fits) acc_q <= sub[16:0];
                    vp_q       <= fits ? vp_next : w8;
                    delta_q[2] <= fits;
                    state      <= S_Q1;
                end
                S_Q1: begin
                    if (fits) acc_q <= sub[16:0];
                    if (fits) vp_q <= vp_next;
                    delta_q[1] <= fits;
                    state      <= S_Q0;
                end
                S_Q0: begin
                    if (fits) vp_q <= vp_next;
                    delta_q[0] <= fits;
                    state      <= S_UPD;
                end
                S_UPD: begin
                    pred_q  <= pnext;
                    index_q <= index_nx;
                    nib_q   <= {sign_q, delta_q};
                    ov_q    <= 1'b1;
                    state   <= S_IDLE;
                end
                default: state <= S_IDLE;
            endcase
        end
    end
endmodule
