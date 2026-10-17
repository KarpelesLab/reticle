// adpcm_ima_dec — IMA/DVI ADPCM the other way: one 4-bit nibble in,
// one 16-bit sample out.
//
// What it does
//   **This half is the specified one.** An encoder may choose any
//   nibble it likes for a sample and the result is still ADPCM; what a
//   decoder does with that nibble is fixed, because every decoder has
//   to agree or the stream means nothing. So this is the module whose
//   output is checked against somebody else's, sample for sample:
//   `adpcm_ima_dec_matches_ffmpegs_decoder` runs ADPCM that ffmpeg
//   encoded through this block and compares every sample with what
//   ffmpeg's own decoder produced.
//
//   The reconstruction is a sum of the three magnitude bits' weights —
//   step, step/2, step/4 — over a floor of step/8, added to or
//   subtracted from the predictor according to the sign bit, clamped to
//   sixteen bits; then the step index moves by -1, 2, 4, 6 or 8
//   according to the magnitude bits and is clamped to 0..88. It is
//   `adpcm_ima_enc`'s state update exactly, which is why the encoder
//   can say what a decoder will produce.
//
// Four cycles a nibble
//   One arithmetic operation per cycle, for the reason
//   `adpcm_ima_enc`'s header gives at length: the sum and the predictor
//   update are a chain of ripple carries, and in one cycle this measured
//   28 logic levels on a four-input fabric and 14 on an ECP5, against
//   the 19 that did not meet 60 MHz in `docs/fpga-trellis.md`. It is 14
//   and 14 as a sequencer. A nibble is taken in
//   `S_IDLE` and the sample is ready three cycles later, which at audio
//   rates is free. The clamp is a test on the top three bits of a
//   19-bit sum rather than a pair of comparators; the encoder's header
//   says why that works.
//
//   `load` seeds the predictor and step index that a WAV block header
//   carries, and is honoured only in `S_IDLE` — a block boundary in the
//   middle of a nibble is not something the format can express. It
//   touches the state and not the output slot.
//
// What it does not do
//   No block framing, no nibble unpacking and no stereo de-interleave:
//   a caller reading a `.wav` splits the blocks, calls `load` with each
//   header, and presents the nibbles two to a byte, low nibble first.
//
//   Nothing about a malformed stream, because there is no such thing
//   here: every one of the sixteen nibble values is legal and every
//   step index this block can reach is in range. A block header that
//   carries an index above 88 is the one input that is out of range,
//   and `load` clamps it rather than reading off the end of the table.
module adpcm_ima_dec (
    input  wire               clk,
    input  wire               rst_n,

    input  wire               load,
    input  wire signed [15:0] load_pred,
    input  wire        [6:0]  load_index,

    input  wire        [3:0]  in_nibble,
    input  wire               in_valid,
    output wire               in_ready,

    output wire signed [15:0] out_sample,
    output wire               out_valid,
    input  wire               out_ready,

    output wire signed [15:0] predictor,
    output wire        [6:0]  step_index
);
    localparam [1:0] S_IDLE = 2'd0;
    localparam [1:0] S_V1   = 2'd1;   // add step/2 if its bit is set
    localparam [1:0] S_V0   = 2'd2;   // add step/4 if its bit is set
    localparam [1:0] S_UPD  = 2'd3;   // the predictor and the index

    reg [1:0]         state;
    reg signed [15:0] pred_q;
    reg        [6:0]  index_q;
    reg        [16:0] vp_q;
    reg        [3:0]  nib_q;
    reg signed [15:0] samp_q;
    reg               ov_q;

    // Registered, so the 89-entry table is never in series with an
    // adder; `adpcm_ima_enc`'s header says what that cost when it was
    // not. `step_q` and `index_q` are latched together and so always
    // agree.
    wire [6:0] index_nx = load             ? ((load_index > 7'd88) ? 7'd88
                                                                  : load_index)
                        : (state == S_UPD) ? inext
                        :                    index_q;
    wire [14:0] step_d;
    adpcm_step u_step (.index(index_nx), .step(step_d));
    reg  [14:0] step_q;
    wire [14:0] step = step_q;

    wire [16:0] w1 = {2'b0, step};
    wire [16:0] w2 = {3'b0, step[14:1]};
    wire [16:0] w4 = {4'b0, step[14:2]};
    wire [16:0] w8 = {5'b0, step[14:3]};

    wire [2:0] delta = nib_q[2:0];
    wire       sign  = nib_q[3];

    // One addend a state. `S_IDLE` forms the floor and the step bit's
    // contribution together, which is one carry because both are
    // functions of `step` alone.
    wire [16:0] vp_v1 = vp_q + w2;
    wire [16:0] vp_v0 = vp_q + w4;

    wire signed [18:0] pwide = sign
        ? ({{3{pred_q[15]}}, pred_q} - {2'b0, vp_q})
        : ({{3{pred_q[15]}}, pred_q} + {2'b0, vp_q});
    wire               pfits = (pwide[18:16] == {3{pwide[15]}});
    wire signed [15:0] pnext = pfits ? pwide[15:0]
                             : (pwide[18] ? 16'sh8000 : 16'sh7FFF);

    wire signed [7:0] adj   = delta[2] ? (({5'd0, delta} - 8'sd3) <<< 1)
                                       : -8'sd1;
    wire signed [8:0] iwide = {2'b0, index_q} + {adj[7], adj};
    wire        [6:0] inext = iwide[8]         ? 7'd0
                            : (iwide > 9'sd88) ? 7'd88
                            :                    iwide[6:0];

    assign in_ready   = (state == S_IDLE) && (!ov_q || out_ready);
    assign out_sample = samp_q;
    assign out_valid  = ov_q;
    assign predictor  = pred_q;
    assign step_index = index_q;

    wire take = in_valid && in_ready;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state   <= S_IDLE;
            pred_q  <= 16'sd0;
            index_q <= 7'd0;
            vp_q    <= 17'd0;
            nib_q   <= 4'd0;
            samp_q  <= 16'sd0;
            ov_q    <= 1'b0;
            step_q  <= 15'd7;
        end else begin
            step_q <= step_d;
            if (ov_q && out_ready && (state != S_UPD)) ov_q <= 1'b0;
            case (state)
                S_IDLE: begin
                    if (load) begin
                        pred_q  <= load_pred;
                        index_q <= index_nx;
                    end else if (take) begin
                        nib_q <= in_nibble;
                        vp_q  <= in_nibble[2] ? (w8 + w1) : w8;
                        state <= S_V1;
                    end
                end
                S_V1: begin
                    if (delta[1]) vp_q <= vp_v1;
                    state <= S_V0;
                end
                S_V0: begin
                    if (delta[0]) vp_q <= vp_v0;
                    state <= S_UPD;
                end
                S_UPD: begin
                    pred_q  <= pnext;
                    index_q <= index_nx;
                    samp_q  <= pnext;
                    ov_q    <= 1'b1;
                    state   <= S_IDLE;
                end
                default: state <= S_IDLE;
            endcase
        end
    end
endmodule
