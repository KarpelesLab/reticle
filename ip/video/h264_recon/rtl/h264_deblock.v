// h264_deblock — the deblocking filter of ITU-T H.264 §8.7, one
// macroblock at a time, and the picture's way out.
//
// What it does
//   On `go`, filters the macroblock in the current slot (`cur_*`) and,
//   across its left edge, the right-hand columns of the one in the left
//   slot (`left_*`), and across its top edge the bottom rows of the one
//   above, which this module keeps in a line buffer of its own. Then it
//   sends on every sample that no later macroblock can change any more,
//   as row records to `h264_frame_wr`.
//
// The order the specification requires, and the order this uses
//   §8.7: per macroblock, luma then each chroma component; vertical
//   edges left to right, then horizontal edges top to bottom. The work
//   here is done a band of four rows at a time — a band's vertical edges,
//   then the horizontal edge on top of it, then the next band — which
//   is the same function: the horizontal edge at row 4b reads rows
//   4b-4 .. 4b+3 and changes 4b-3 .. 4b+2, and the vertical edges of
//   any band below that touch only their own rows. Within a band the
//   vertical edges go edge by edge across all four rows, so a line's
//   edge e+1 always sees its edge e done.
//
// Registers
//   Q holds the band being filtered (four rows of sixteen samples), P
//   the band above it (the macroblock's previous band, or for band 0 the
//   macroblock above's rows 12..15 from the line buffer), and L the four
//   samples to the left of each of Q's rows (the left macroblock's
//   columns 12..15). Chroma uses the same registers eight samples wide:
//   rows 0..3 and 4..7 are its two bands, the left macroblock gives
//   columns 4..7, and the macroblock above gives rows 6 and 7 as P's
//   rows 2 and 3 — the chroma filter reads only p1, p0, q0 and q1.
//
//   An edge's line goes to `h264_dbf_filter` from a register stage that
//   picks its eight samples and looks up its thresholds, and its result
//   is written back two cycles later. Consecutive lines are different
//   rows (vertical edges) or columns (horizontal), so nothing waits.
//
// What leaves, and when
//   A sample is final once nothing after it can filter it. After this
//   macroblock: the left macroblock's rows 0..11 (its columns 13..15
//   were the last thing to change, just now); the above macroblock's
//   rows 12..15 (its top edge was filtered just now); and, at the end of
//   a row, this macroblock's rows 0..11 too. Rows 12..15 of a macroblock
//   go to the line buffer instead, to be filtered as P and sent by the
//   macroblock below — except in the last row of the picture, where
//   nothing comes below and all sixteen rows go at once. Chroma likewise
//   with rows 0..5 and 6..7.
//
// The parameters of the macroblock (all held from `go` to `done`)
//   `bs` holds 32 boundary strengths, three bits each: vertical edge e
//   (x = 4e) segment k (rows 4k..4k+3) at [3 * (4e + k) +: 3], horizontal
//   edge e (y = 4e) segment k (columns 4k..4k+3) at [48 + 3 * (4e + k) +: 3],
//   already derived by 8.7.2.1 upstream. A chroma edge takes the bS of
//   the luma edge through the luma sample (2x, 2y) (8.7.2's chroma rule):
//   chroma edge 0 is luma edge 0, chroma edge 1 (x or y = 4) luma edge 2,
//   and chroma line l segment l / 2.
//   `filt_*` are filterLeftMbEdgeFlag, filterTopMbEdgeFlag and
//   filterInternalEdgesFlag (8.7 step 2), with the picture edges and
//   disable_deblocking_filter_idc already folded in.
//   The QPs are each macroblock's QPY (0 for I_PCM) and the QPC that
//   8.5.8 gives for it in each chroma component, for this macroblock and
//   the ones to the left and above; `off_a` and `off_b` are FilterOffsetA
//   and FilterOffsetB of this macroblock's slice (8.7.2: "the slice that
//   contains ... q0").
//   `t8x8` is transform_size_8x8_flag: luma edges 1 and 3 are left
//   unfiltered when it is set (8.7, the dashed edges). Nothing upstream
//   sets it yet.
//
// Output records (`op_*`, 34 bits)
//   [33:32] = 1, a header: [31:28] frame, [27:26] plane (0 Y, 1 Cb, 2 Cr),
//           [25:15] y, [14:4] x of the first sample, [2:0] words - 1;
//   [33:32] = 0, a word of four samples, the first lowest;
//   [33:32] = 2, the end of the picture, after its last word.
//   A record is started only when `op_room` says eight entries are free.
module h264_deblock #(
    parameter integer MAX_W_MBS = 80
) (
    input  wire        clk,
    input  wire        rst_n,

    input  wire        go,
    output wire        busy,
    output reg         done,

    input  wire [6:0]  mb_x,
    input  wire [6:0]  mb_y,
    input  wire        last_col,
    input  wire        last_row,
    input  wire [3:0]  frame,
    input  wire        filt_left,
    input  wire        filt_top,
    input  wire        filt_inner,
    input  wire        t8x8,
    input  wire [95:0] bs,
    input  wire [5:0]  qp_y,
    input  wire [5:0]  qp_y_left,
    input  wire [5:0]  qp_y_top,
    input  wire [5:0]  qp_cb,
    input  wire [5:0]  qp_cb_left,
    input  wire [5:0]  qp_cb_top,
    input  wire [5:0]  qp_cr,
    input  wire [5:0]  qp_cr_left,
    input  wire [5:0]  qp_cr_top,
    input  wire signed [4:0] off_a,
    input  wire signed [4:0] off_b,

    // The current macroblock's slot and the left one's: a read port
    // answering a cycle later, and a write port.
    output reg  [6:0]  cur_ra,      // combinational: data the next cycle
    input  wire [31:0] cur_rd,
    output reg         cur_we,
    output reg  [6:0]  cur_wa,
    output reg  [31:0] cur_wd,
    output reg  [6:0]  left_ra,
    input  wire [31:0] left_rd,
    output reg         left_we,
    output reg  [6:0]  left_wa,
    output reg  [31:0] left_wd,

    // Row records for the frame writer.
    output reg         op_valid,
    output reg  [33:0] op_data,
    input  wire        op_room
);
    // -----------------------------------------------------------------
    // The line buffer: luma rows 12..15 and chroma rows 6..7 of every
    // macroblock of the row above, a word per four samples.
    // -----------------------------------------------------------------
    localparam integer LW    = MAX_W_MBS * 4;      // luma words a row
    localparam integer CW    = MAX_W_MBS * 2;      // chroma words a row
    localparam integer LB_CB = 4 * LW;
    localparam integer LB_CR = 4 * LW + 2 * CW;
    localparam integer LB_N  = 4 * LW + 4 * CW;
    localparam [10:0] LW11    = LW;
    localparam [10:0] CW11    = CW;
    localparam [10:0] LB_CB11 = LB_CB;
    localparam [10:0] LB_CR11 = LB_CR;

    reg [31:0] lbuf [0:LB_N-1];
    reg [31:0] lb_q;
    reg        lb_we;
    reg [10:0] lb_wa;
    reg [31:0] lb_wd;
    reg [10:0] lb_ra;             // combinational, like cur_ra and left_ra

    integer n;
    initial begin
        for (n = 0; n < LB_N; n = n + 1) lbuf[n] = 32'd0;
    end

    always @(posedge clk) begin
        if (lb_we) lbuf[lb_wa] <= lb_wd;
        lb_q <= lbuf[lb_ra];
    end

    // Where row `r` of a component of the macroblock at column `x`
    // lives in the line buffer (luma r = 0..3 for rows 12..15, chroma
    // r = 0..1 for rows 6..7), word `w` of it.
    function [10:0] lb_addr;
        input [1:0] plane;
        input [1:0] r;
        input [6:0] x;
        input [1:0] w;
        begin
            case (plane)
                2'd0:    lb_addr = {2'd0, r} * LW11 + {2'd0, x, w};
                2'd1:    lb_addr = LB_CB11 + {10'd0, r[0]} * CW11 + {3'd0, x, w[0]};
                default: lb_addr = LB_CR11 + {10'd0, r[0]} * CW11 + {3'd0, x, w[0]};
            endcase
        end
    endfunction

    // -----------------------------------------------------------------
    // The band registers
    // -----------------------------------------------------------------
    // Row r of Q at [128 r +: 128], sample x of it at [8 x +: 8]; P alike;
    // L row r at [32 r +: 32], its sample i (column 12 + i, or 4 + i in
    // chroma) at [8 i +: 8].
    reg [511:0] qb;
    reg [511:0] pb;
    reg [127:0] lr;

    // -----------------------------------------------------------------
    // Control
    // -----------------------------------------------------------------
    localparam [3:0] PH_IDLE  = 4'd0;
    localparam [3:0] PH_XFER  = 4'd1;   // move Q to P, write P back, load Q, L (and P)
    localparam [3:0] PH_V     = 4'd2;   // vertical edges of the band
    localparam [3:0] PH_H     = 4'd3;   // the horizontal edge above it; L goes back
    localparam [3:0] PH_EMITP = 4'd4;   // send the macroblock above's bottom rows
    localparam [3:0] PH_FIN   = 4'd5;   // write the last two bands back
    localparam [3:0] PH_EMIT  = 4'd6;   // send what is final
    localparam [3:0] PH_EOP   = 4'd7;
    localparam [3:0] PH_DONE  = 4'd8;

    reg [3:0] ph;
    reg [1:0] plane;          // 0 Y, 1 Cb, 2 Cr
    reg [1:0] band;
    reg [5:0] t;              // step inside a phase
    reg       emit_cur;       // PH_EMIT: sending the current macroblock (else the left)

    assign busy = (ph != PH_IDLE);

    wire       luma   = (plane == 2'd0);
    wire [1:0] nbands = luma ? 2'd3 : 2'd1;          // last band index
    wire [4:0] nwords = luma ? 5'd16 : 5'd8;          // words a band
    wire       has_left = (mb_x != 7'd0);
    wire       has_top  = (mb_y != 7'd0);

    // Slot word of row r (of the plane), word w.
    function [6:0] slot_addr;
        input [1:0] p;
        input [3:0] r;
        input [1:0] w;
        begin
            case (p)
                2'd0:    slot_addr = {1'b0, r, w};
                2'd1:    slot_addr = 7'd64 + {2'd0, r[2:0], w[0]};
                default: slot_addr = 7'd80 + {2'd0, r[2:0], w[0]};
            endcase
        end
    endfunction

    // Band word i (0..15 luma, 0..7 chroma): its row and word.
    wire [5:0] tm1 = t - 6'd1;
    function [1:0] bw_row;
        input       lum;
        input [3:0] i;
        begin
            bw_row = lum ? i[3:2] : i[2:1];
        end
    endfunction
    function [1:0] bw_w;
        input       lum;
        input [3:0] i;
        begin
            bw_w = lum ? i[1:0] : {1'b0, i[0]};
        end
    endfunction

    // A band word of a band register.
    function [31:0] band_word;
        input [511:0] v;
        input [1:0]   r;
        input [1:0]   w;
        begin
            band_word = v[128 * r + 32 * w +: 32];
        end
    endfunction

    // -----------------------------------------------------------------
    // The edge operations
    // -----------------------------------------------------------------
    // Op counter: luma vertical e = oc[3:2], r = oc[1:0]; luma horizontal
    // column oc; chroma vertical e = oc[2], r = oc[1:0]; chroma
    // horizontal column oc[2:0].
    wire        op_issue = (ph == PH_V || ph == PH_H) && (t < {1'b0, nwords});
    wire [3:0]  oc       = t[3:0];
    wire        op_vert  = (ph == PH_V);
    wire [1:0]  op_e     = luma ? oc[3:2] : {1'b0, oc[2]};
    wire [1:0]  op_r     = oc[1:0];
    wire [3:0]  op_c     = luma ? oc : {1'b0, oc[2:0]};

    // The edge's luma edge index and the segment the line is in.
    wire [1:0] e_luma = luma ? (op_vert ? op_e : band) : (op_vert ? {op_e[0], 1'b0} : {band[0], 1'b0});
    wire [1:0] seg    = luma ? (op_vert ? band : op_c[3:2])
                             : (op_vert ? {band[0], op_r[1]} : op_c[2:1]);
    wire [6:0] bs_bit = op_vert ? (7'd3 * {3'd0, e_luma, seg}) : (7'd48 + 7'd3 * {3'd0, e_luma, seg});
    wire [2:0] op_bs_raw = bs[bs_bit +: 3];

    // Is this edge filtered at all?
    wire mb_edge  = op_vert ? (op_e == 2'd0) : (band == 2'd0);
    wire edge_on  = mb_edge ? (op_vert ? filt_left : filt_top)
                            : (filt_inner && !(luma && t8x8 && (op_vert ? op_e[0] : band[0])));
    wire [2:0] op_bs = edge_on ? op_bs_raw : 3'd0;

    // qPav (8-453): with the left or top macroblock on a macroblock edge.
    wire [5:0] qp_q = (plane == 2'd0) ? qp_y : (plane == 2'd1) ? qp_cb : qp_cr;
    wire [5:0] qp_p = !mb_edge ? qp_q :
                      op_vert ? ((plane == 2'd0) ? qp_y_left : (plane == 2'd1) ? qp_cb_left : qp_cr_left)
                              : ((plane == 2'd0) ? qp_y_top  : (plane == 2'd1) ? qp_cb_top  : qp_cr_top);
    wire [6:0] qpav = ({1'b0, qp_p} + {1'b0, qp_q} + 7'd1) >> 1;
    wire signed [7:0] ia_s = $signed({1'b0, qpav}) + {{3{off_a[4]}}, off_a};
    wire signed [7:0] ib_s = $signed({1'b0, qpav}) + {{3{off_b[4]}}, off_b};
    // 8-454 / 8-455.
    wire [5:0] index_a = ia_s[7] ? 6'd0 : ((ia_s > 8'sd51) ? 6'd51 : ia_s[5:0]);
    wire [5:0] index_b = ib_s[7] ? 6'd0 : ((ib_s > 8'sd51) ? 6'd51 : ib_s[5:0]);

    wire [7:0] tb_alpha;
    wire [4:0] tb_beta;
    wire [4:0] tb_tc0;
    h264_dbf_tables tables (
        .index_a(index_a), .index_b(index_b), .bs(op_bs),
        .alpha(tb_alpha), .beta(tb_beta), .tc0(tb_tc0)
    );

    // The eight samples of the line: a window of a row of {Q, L}, or a
    // column of P over Q.
    wire [159:0] xrow = {qb[128 * op_r +: 128], lr[32 * op_r +: 32]};
    wire [63:0]  vwin = xrow[32 * op_e +: 64];
    wire [63:0]  hwin = {qb[384 + 8 * op_c +: 8], qb[256 + 8 * op_c +: 8],
                         qb[128 + 8 * op_c +: 8], qb[8 * op_c +: 8],
                         pb[384 + 8 * op_c +: 8], pb[256 + 8 * op_c +: 8],
                         pb[128 + 8 * op_c +: 8], pb[8 * op_c +: 8]};
    wire [63:0]  win  = op_vert ? vwin : hwin;     // p3 lowest, q3 highest

    // Stage 1: the line and its thresholds.
    reg        s1_v, s1_vert, s1_chroma;
    reg [1:0]  s1_e, s1_r;
    reg [3:0]  s1_c;
    reg [63:0] s1_win;
    reg [2:0]  s1_bs;
    reg [7:0]  s1_alpha;
    reg [4:0]  s1_beta, s1_tc0;

    wire [7:0] f_p2, f_p1, f_p0, f_q0, f_q1, f_q2;
    h264_dbf_filter filter (
        .p3(s1_win[7:0]),   .p2(s1_win[15:8]),  .p1(s1_win[23:16]), .p0(s1_win[31:24]),
        .q0(s1_win[39:32]), .q1(s1_win[47:40]), .q2(s1_win[55:48]), .q3(s1_win[63:56]),
        .bs(s1_bs), .chroma(s1_chroma),
        .alpha(s1_alpha), .beta(s1_beta), .tc0(s1_tc0),
        .p2o(f_p2), .p1o(f_p1), .p0o(f_p0), .q0o(f_q0), .q1o(f_q1), .q2o(f_q2)
    );

    // Stage 2: the six results, and where they go.
    reg        s2_v, s2_vert;
    reg [1:0]  s2_e, s2_r;
    reg [3:0]  s2_c;
    reg [47:0] s2_out;          // p2 lowest .. q2 highest

    // -----------------------------------------------------------------
    // Emission
    // -----------------------------------------------------------------
    // PH_EMIT walks rows: luma 0..15, Cb 0..7, Cr 0..7 (32 rows); each
    // either goes out (header and words) or into the line buffer.
    reg [5:0]  er;            // emission row, all planes
    reg [2:0]  ek;            // step inside the row
    wire [1:0] e_plane = er[5:4] == 2'b00 ? 2'd0 : (er[3] ? 2'd2 : 2'd1);
    wire [3:0] e_row   = (e_plane == 2'd0) ? er[3:0] : {1'b0, er[2:0]};
    wire [2:0] e_nw    = (e_plane == 2'd0) ? 3'd4 : 3'd2;
    wire       e_tolb  = !last_row && ((e_plane == 2'd0) ? (e_row >= 4'd12) : (e_row >= 4'd6));
    wire [6:0] e_mbx   = emit_cur ? mb_x : (mb_x - 7'd1);
    wire [1:0] e_lbr   = (e_plane == 2'd0) ? (e_row[1:0]) : {1'b0, e_row[0]};

    // A header for row `row` of `p` at macroblock column `x`, macroblock
    // row `y` (in units of its own plane).
    function [33:0] header;
        input [3:0]  fr;
        input [1:0]  p;
        input [6:0]  x;
        input [10:0] yrow;
        begin
            header = {2'b01, fr, p, yrow,
                      (p == 2'd0) ? {x, 4'd0} : {1'b0, x, 3'd0},
                      1'b0, (p == 2'd0) ? 3'd3 : 3'd1};
        end
    endfunction

    wire [2:0]  e_pw = luma ? 3'd4 : 3'd2;      // words a row of P
    wire [3:0]  fin_u = t[3:0] - nwords[3:0];   // PH_FIN: Q's word

    // The read ports, answering the cycle after.
    always @* begin
        cur_ra  = 7'd0;
        left_ra = 7'd0;
        lb_ra   = 11'd0;
        if (ph == PH_XFER) begin
            cur_ra  = slot_addr(plane, {band, bw_row(luma, t[3:0])}, bw_w(luma, t[3:0]));
            left_ra = slot_addr(plane, {band, t[1:0]}, luma ? 2'd3 : 2'd1);
            lb_ra   = luma ? lb_addr(2'd0, t[3:2], mb_x, t[1:0])
                           : lb_addr(plane, {1'b0, t[1]}, mb_x, {1'b0, t[0]});
        end else if (ph == PH_EMIT) begin
            // Word ek of the row (ek = 0 reads word 0 as the header goes).
            cur_ra  = slot_addr(e_plane, e_row, (ek == 3'd0) ? 2'd0 : ek[1:0]);
            left_ra = cur_ra;
        end
    end

    wire [10:0] my16 = {mb_y, 4'd0};
    wire [10:0] my8  = {1'b0, mb_y, 3'd0};

    // -----------------------------------------------------------------
    // The sequence
    // -----------------------------------------------------------------
    integer j;
    reg [159:0] xnew;
    reg [511:0] qn, pn;
    reg [127:0] ln;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            ph <= PH_IDLE; plane <= 2'd0; band <= 2'd0; t <= 6'd0; emit_cur <= 1'b0;
            done <= 1'b0;
            qb <= 512'd0; pb <= 512'd0; lr <= 128'd0;
            s1_v <= 1'b0; s1_vert <= 1'b0; s1_chroma <= 1'b0; s1_e <= 2'd0; s1_r <= 2'd0;
            s1_c <= 4'd0; s1_win <= 64'd0; s1_bs <= 3'd0; s1_alpha <= 8'd0;
            s1_beta <= 5'd0; s1_tc0 <= 5'd0;
            s2_v <= 1'b0; s2_vert <= 1'b0; s2_e <= 2'd0; s2_r <= 2'd0; s2_c <= 4'd0;
            s2_out <= 48'd0;
            er <= 6'd0; ek <= 3'd0;
            cur_we <= 1'b0; cur_wa <= 7'd0; cur_wd <= 32'd0;
            left_we <= 1'b0; left_wa <= 7'd0; left_wd <= 32'd0;
            lb_we <= 1'b0; lb_wa <= 11'd0; lb_wd <= 32'd0;
            op_valid <= 1'b0; op_data <= 34'd0;
        end else begin
            done     <= 1'b0;
            cur_we   <= 1'b0;
            left_we  <= 1'b0;
            lb_we    <= 1'b0;
            op_valid <= 1'b0;

            qn = qb;
            pn = pb;
            ln = lr;

            // ---- the edge pipeline ----
            s1_v <= op_issue;
            if (op_issue) begin
                s1_vert   <= op_vert;
                s1_chroma <= !luma;
                s1_e      <= op_e;
                s1_r      <= op_r;
                s1_c      <= op_c;
                s1_win    <= win;
                s1_bs     <= op_bs;
                s1_alpha  <= tb_alpha;
                s1_beta   <= tb_beta;
                s1_tc0    <= tb_tc0;
            end
            s2_v <= s1_v;
            if (s1_v) begin
                s2_vert <= s1_vert;
                s2_e    <= s1_e;
                s2_r    <= s1_r;
                s2_c    <= s1_c;
                s2_out  <= {f_q2, f_q1, f_q0, f_p0, f_p1, f_p2};
            end
            if (s2_v) begin
                if (s2_vert) begin
                    // Bytes 4e+1 .. 4e+6 of the row {Q, L}.
                    xnew = {qn[128 * s2_r +: 128], ln[32 * s2_r +: 32]};
                    for (j = 0; j < 6; j = j + 1)
                        xnew[8 * (4 * s2_e + 1 + j) +: 8] = s2_out[8 * j +: 8];
                    qn[128 * s2_r +: 128] = xnew[159:32];
                    ln[32 * s2_r +: 32]   = xnew[31:0];
                end else begin
                    pn[128 + 8 * s2_c +: 8] = s2_out[7:0];
                    pn[256 + 8 * s2_c +: 8] = s2_out[15:8];
                    pn[384 + 8 * s2_c +: 8] = s2_out[23:16];
                    qn[8 * s2_c +: 8]       = s2_out[31:24];
                    qn[128 + 8 * s2_c +: 8] = s2_out[39:32];
                    qn[256 + 8 * s2_c +: 8] = s2_out[47:40];
                end
            end

            case (ph)
                PH_IDLE: if (go) begin
                    ph    <= PH_XFER;
                    plane <= 2'd0;
                    band  <= 2'd0;
                    t     <= 6'd0;
                end

                // Cycle t issues the reads of band word t; cycle t + 1
                // lands them, moving that word of Q to P (and writing
                // P's old word back) on the way. Band 0 takes P from the
                // line buffer instead, when there is a row above.
                PH_XFER: begin
                    t <= t + 6'd1;
                    if (t != 6'd0) begin
                        // Band word tm1: row bw_row, word bw_w.
                        if (band >= 2'd2) begin
                            cur_we <= 1'b1;
                            cur_wa <= slot_addr(plane, {band - 2'd2, bw_row(luma, tm1[3:0])},
                                                bw_w(luma, tm1[3:0]));
                            cur_wd <= band_word(pb, bw_row(luma, tm1[3:0]), bw_w(luma, tm1[3:0]));
                        end
                        if (band != 2'd0) begin
                            pn[128 * bw_row(luma, tm1[3:0]) + 32 * bw_w(luma, tm1[3:0]) +: 32] =
                                band_word(qb, bw_row(luma, tm1[3:0]), bw_w(luma, tm1[3:0]));
                        end else if (has_top) begin
                            if (luma)
                                pn[32 * tm1[3:0] +: 32] = lb_q;
                            else if (tm1 < 6'd4)
                                // Above rows 6 and 7 are P's rows 2 and 3.
                                pn[256 + 128 * tm1[1] + 32 * tm1[0] +: 32] = lb_q;
                        end
                        qn[128 * bw_row(luma, tm1[3:0]) + 32 * bw_w(luma, tm1[3:0]) +: 32] = cur_rd;
                        if (tm1 < 6'd4)
                            ln[32 * tm1[1:0] +: 32] = left_rd;
                    end
                    if (t == {1'b0, nwords}) begin
                        ph <= PH_V;
                        t  <= 6'd0;
                    end
                end

                PH_V: begin
                    t <= t + 6'd1;
                    // Two cycles after the last line for its result to land.
                    if (t == {1'b0, nwords} + 6'd2) begin
                        ph <= PH_H;
                        t  <= 6'd0;
                    end
                end

                // The horizontal edge on top of the band; meanwhile the
                // left macroblock's columns go back to its slot.
                PH_H: begin
                    t <= t + 6'd1;
                    if (t < 6'd4 && has_left) begin
                        left_we <= 1'b1;
                        left_wa <= slot_addr(plane, {band, t[1:0]}, luma ? 2'd3 : 2'd1);
                        left_wd <= lr[32 * t[1:0] +: 32];
                    end
                    if (t == {1'b0, nwords} + 6'd2) begin
                        t <= 6'd0;
                        if (band == 2'd0 && has_top) begin
                            ph <= PH_EMITP;
                            er <= 6'd0;
                            ek <= 3'd0;
                        end else if (band == nbands) begin
                            ph <= PH_FIN;
                        end else begin
                            band <= band + 2'd1;
                            ph   <= PH_XFER;
                        end
                    end
                end

                // The macroblock above's bottom rows, final now: luma P
                // rows 0..3 (its rows 12..15), chroma P rows 2..3 (6..7).
                // Five steps a row: the header, then the words.
                PH_EMITP: begin
                    if (ek == 3'd0) begin
                        if (op_room) begin
                            op_valid <= 1'b1;
                            op_data  <= luma ?
                                header(frame, 2'd0, mb_x, my16 - 11'd4 + {9'd0, er[1:0]}) :
                                header(frame, plane, mb_x, my8 - 11'd2 + {10'd0, er[0]});
                            ek <= 3'd1;
                        end
                    end else begin
                        op_valid <= 1'b1;
                        op_data  <= {2'b00, luma ? band_word(pb, er[1:0], ek[1:0] - 2'd1)
                                                 : band_word(pb, {1'b1, er[0]}, ek[1:0] - 2'd1)};
                        if (ek == e_pw) begin
                            ek <= 3'd0;
                            er <= er + 6'd1;
                            if (er == (luma ? 6'd3 : 6'd1)) begin
                                er <= 6'd0;
                                if (band == nbands) begin
                                    ph <= PH_FIN;
                                end else begin
                                    band <= band + 2'd1;
                                    ph   <= PH_XFER;
                                end
                            end
                        end else begin
                            ek <= ek + 3'd1;
                        end
                    end
                end

                // P (the band before last) and Q (the last) go back.
                PH_FIN: begin
                    t <= t + 6'd1;
                    if (t < {1'b0, nwords}) begin
                        if (nbands != 2'd0) begin
                            cur_we <= 1'b1;
                            cur_wa <= slot_addr(plane, {nbands - 2'd1, bw_row(luma, t[3:0])},
                                                bw_w(luma, t[3:0]));
                            cur_wd <= band_word(pb, bw_row(luma, t[3:0]), bw_w(luma, t[3:0]));
                        end
                    end else begin
                        cur_we <= 1'b1;
                        cur_wa <= slot_addr(plane, {nbands, bw_row(luma, fin_u)}, bw_w(luma, fin_u));
                        cur_wd <= band_word(qb, bw_row(luma, fin_u), bw_w(luma, fin_u));
                        if (t == {nwords, 1'b0} - 6'd1) begin
                            t    <= 6'd0;
                            band <= 2'd0;
                            if (plane == 2'd2) begin
                                er <= 6'd0;
                                ek <= 3'd0;
                                if (has_left) begin
                                    ph       <= PH_EMIT;
                                    emit_cur <= 1'b0;
                                end else if (last_col) begin
                                    ph       <= PH_EMIT;
                                    emit_cur <= 1'b1;
                                end else begin
                                    ph <= PH_DONE;
                                end
                            end else begin
                                plane <= plane + 2'd1;
                                ph    <= PH_XFER;
                            end
                        end
                    end
                end

                // What is final, from a slot: a row either goes out as a
                // header and its words, or into the line buffer.
                PH_EMIT: begin
                    if (ek == 3'd0) begin
                        if (e_tolb || op_room) begin
                            if (!e_tolb) begin
                                op_valid <= 1'b1;
                                op_data  <= header(frame, e_plane, e_mbx,
                                                   (e_plane == 2'd0) ? (my16 + {7'd0, e_row})
                                                                     : (my8 + {7'd0, e_row}));
                            end
                            ek <= 3'd1;
                        end
                    end else begin
                        // Word ek - 1 has landed.
                        if (e_tolb) begin
                            lb_we <= 1'b1;
                            lb_wa <= lb_addr(e_plane, e_lbr, e_mbx, ek[1:0] - 2'd1);
                            lb_wd <= emit_cur ? cur_rd : left_rd;
                        end else begin
                            op_valid <= 1'b1;
                            op_data  <= {2'b00, emit_cur ? cur_rd : left_rd};
                        end
                        if (ek == e_nw) begin
                            ek <= 3'd0;
                            er <= er + 6'd1;
                            if (er == 6'd31) begin
                                er <= 6'd0;
                                if (!emit_cur && last_col) begin
                                    emit_cur <= 1'b1;
                                end else if (last_col && last_row) begin
                                    ph <= PH_EOP;
                                end else begin
                                    ph <= PH_DONE;
                                end
                            end
                        end else begin
                            ek <= ek + 3'd1;
                        end
                    end
                end

                PH_EOP: if (op_room) begin
                    op_valid <= 1'b1;
                    op_data  <= {2'b10, 32'd0};
                    ph       <= PH_DONE;
                end

                PH_DONE: begin
                    done <= 1'b1;
                    ph   <= PH_IDLE;
                end

                default: ph <= PH_IDLE;
            endcase

            qb <= qn;
            pb <= pn;
            lr <= ln;
        end
    end
endmodule
