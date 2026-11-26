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
    // Rows are 512 words apart (luma) and 256 (chroma), powers of two,
    // so an address is bits put side by side: up to 128 macroblocks
    // across, of which a 1280-sample picture uses 80.
    localparam integer LB_N = 3072;
    reg [31:0] lbuf [0:LB_N-1];
    reg [31:0] lb_q;
    reg        lb_we;
    reg [11:0] lb_wa;
    reg [31:0] lb_wd;
    reg [11:0] lb_ra;             // combinational, like cur_ra and left_ra

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
    function [11:0] lb_addr;
        input [1:0] plane;
        input [1:0] r;
        input [6:0] x;
        input [1:0] w;
        begin
            case (plane)
                2'd0:    lb_addr = {1'b0, r, x, w};
                2'd1:    lb_addr = {3'b100, r[0], x, w[0]};
                default: lb_addr = {3'b101, r[0], x, w[0]};
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

    // A band word of a band register. A case, not `v[128 * r + 32 * w
    // +: 32]`: this flow builds a part-select at a computed offset as a
    // shifter across the whole 512 bits, which for three of these was
    // most of the block.
    function [31:0] band_word;
        input [511:0] v;
        input [1:0]   r;
        input [1:0]   w;
        begin
            case ({r, w})
                4'd0:  band_word = v[31:0];
                4'd1:  band_word = v[63:32];
                4'd2:  band_word = v[95:64];
                4'd3:  band_word = v[127:96];
                4'd4:  band_word = v[159:128];
                4'd5:  band_word = v[191:160];
                4'd6:  band_word = v[223:192];
                4'd7:  band_word = v[255:224];
                4'd8:  band_word = v[287:256];
                4'd9:  band_word = v[319:288];
                4'd10: band_word = v[351:320];
                4'd11: band_word = v[383:352];
                4'd12: band_word = v[415:384];
                4'd13: band_word = v[447:416];
                4'd14: band_word = v[479:448];
                default: band_word = v[511:480];
            endcase
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
    reg  [2:0] op_bs_raw;
    integer bi;
    always @* begin
        op_bs_raw = 3'd0;
        for (bi = 0; bi < 32; bi = bi + 1)
            if ({!op_vert, e_luma, seg} == bi) op_bs_raw = bs[3 * bi +: 3];
    end

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

    // The eight samples of the line. The registers turn under the
    // edges rather than the edges reaching into the registers: each
    // vertical-edge line rotates the rows of Q and L up by one, so the
    // line is always taken from row 0, and each horizontal-edge line
    // rotates the columns of P and Q left by one (sixteen of them, or
    // chroma's eight), so it is always column 0. A pass of 16 (or 8)
    // lines turns the band all the way round. Picking a line from any
    // row and any column of 144 bytes, and putting its result back
    // anywhere, was most of this block's logic when it was written
    // that way: 17 700 lookup tables, against what this costs.
    wire [159:0] xrow = {qb[127:0], lr[31:0]};
    reg  [63:0]  vwin;
    always @* begin
        case (op_e)
            2'd0:    vwin = xrow[63:0];
            2'd1:    vwin = xrow[95:32];
            2'd2:    vwin = xrow[127:64];
            default: vwin = xrow[159:96];
        endcase
    end
    wire [63:0]  hwin = {qb[391:384], qb[263:256], qb[135:128], qb[7:0],
                         pb[391:384], pb[263:256], pb[135:128], pb[7:0]};
    wire [63:0]  win  = op_vert ? vwin : hwin;     // p3 lowest, q3 highest
    wire         rot_v = op_issue && op_vert;
    wire         rot_h = op_issue && !op_vert;

    // Stage 1: the line and its thresholds.
    reg        s1_v, s1_vert, s1_chroma;
    reg [1:0]  s1_e, s1_r;
    reg [3:0]  s1_c;
    reg [1:0]  s1_k;            // rotations since the line was taken
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
    reg [1:0]  s2_k;
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
        lb_ra   = 12'd0;
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
    // -----------------------------------------------------------------
    // The band registers' next values, a word or a byte at a time with
    // every index a constant: what lands from a read in PH_XFER, and
    // what an edge's line writes back.
    // -----------------------------------------------------------------
    reg [511:0] qn, pn;
    reg [127:0] ln;
    integer r, w, c, j;

    // PH_XFER: band word tm1 lands this cycle.
    wire       x_land  = (ph == PH_XFER) && (t != 6'd0);
    wire [1:0] x_row   = bw_row(luma, tm1[3:0]);
    wire [1:0] x_w     = bw_w(luma, tm1[3:0]);
    wire       x_lb    = x_land && (band == 2'd0) && has_top && (luma || (tm1 < 6'd4));
    wire [1:0] x_lbrow = luma ? tm1[3:2] : {1'b1, tm1[1]};
    wire [1:0] x_lbw   = luma ? tm1[1:0] : {1'b0, tm1[0]};
    wire       v_wb    = s2_v && s2_vert;
    wire       h_wb    = s2_v && !s2_vert;

    // Where the line taken k rotations ago is now: row 4 - k, column
    // 16 - k (8 - k for chroma).
    wire [1:0] wb_k = s2_k + {1'b0, s2_vert ? rot_v : rot_h};
    reg [511:0] qr, pr;
    reg [127:0] lrr;

    always @* begin
        // Turn.
        qr  = qb;
        pr  = pb;
        lrr = lr;
        if (rot_v) begin
            qr  = {qb[127:0], qb[511:128]};
            lrr = {lr[31:0], lr[127:32]};
        end
        if (rot_h) begin
            for (r = 0; r < 4; r = r + 1) begin
                if (luma) begin
                    qr[128 * r +: 128] = {qb[128 * r +: 8], qb[128 * r + 8 +: 120]};
                    pr[128 * r +: 128] = {pb[128 * r +: 8], pb[128 * r + 8 +: 120]};
                end else begin
                    qr[128 * r +: 64] = {qb[128 * r +: 8], qb[128 * r + 8 +: 56]};
                    pr[128 * r +: 64] = {pb[128 * r +: 8], pb[128 * r + 8 +: 56]};
                end
            end
        end
        qn = qr;
        pn = pr;
        ln = lrr;
        // Put results back where their line has turned to.
        for (r = 1; r < 4; r = r + 1) begin
            for (c = 0; c < 4; c = c + 1) begin
                for (j = 0; j < 6; j = j + 1) begin
                    if (v_wb && wb_k == 4 - r && s2_e == c) begin
                        if (4 * c + 1 + j < 4)
                            ln[32 * r + 8 * (4 * c + 1 + j) +: 8] = s2_out[8 * j +: 8];
                        else
                            qn[128 * r + 8 * (4 * c + j - 3) +: 8] = s2_out[8 * j +: 8];
                    end
                end
            end
        end
        for (c = 5; c < 16; c = c + 1) begin
            if (h_wb && ((luma && c >= 13 && wb_k == 16 - c) ||
                         (!luma && c >= 5 && c <= 7 && wb_k == 8 - c))) begin
                for (r = 0; r < 4; r = r + 1) begin
                    if (r >= 1) pn[128 * r + 8 * c +: 8] = s2_out[8 * (r - 1) +: 8];
                    if (r <= 2) qn[128 * r + 8 * c +: 8] = s2_out[24 + 8 * r +: 8];
                end
            end
        end
        // What lands in PH_XFER (no line is in flight then).
        for (r = 0; r < 4; r = r + 1) begin
            for (w = 0; w < 4; w = w + 1) begin
                if (x_land && x_row == r && x_w == w)
                    qn[128 * r + 32 * w +: 32] = cur_rd;
                if (x_land && band != 2'd0 && x_row == r && x_w == w)
                    pn[128 * r + 32 * w +: 32] = qb[128 * r + 32 * w +: 32];
                if (x_lb && x_lbrow == r && x_lbw == w)
                    pn[128 * r + 32 * w +: 32] = lb_q;
            end
            if (x_land && tm1 < 6'd4 && tm1[1:0] == r)
                ln[32 * r +: 32] = left_rd;
        end
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            ph <= PH_IDLE; plane <= 2'd0; band <= 2'd0; t <= 6'd0; emit_cur <= 1'b0;
            done <= 1'b0;
            qb <= 512'd0; pb <= 512'd0; lr <= 128'd0;
            s1_v <= 1'b0; s1_vert <= 1'b0; s1_chroma <= 1'b0; s1_e <= 2'd0; s1_r <= 2'd0;
            s1_c <= 4'd0; s1_k <= 2'd0; s2_k <= 2'd0; s1_win <= 64'd0; s1_bs <= 3'd0; s1_alpha <= 8'd0;
            s1_beta <= 5'd0; s1_tc0 <= 5'd0;
            s2_v <= 1'b0; s2_vert <= 1'b0; s2_e <= 2'd0; s2_r <= 2'd0; s2_c <= 4'd0;
            s2_out <= 48'd0;
            er <= 6'd0; ek <= 3'd0;
            cur_we <= 1'b0; cur_wa <= 7'd0; cur_wd <= 32'd0;
            left_we <= 1'b0; left_wa <= 7'd0; left_wd <= 32'd0;
            lb_we <= 1'b0; lb_wa <= 12'd0; lb_wd <= 32'd0;
            op_valid <= 1'b0; op_data <= 34'd0;
        end else begin
            done     <= 1'b0;
            cur_we   <= 1'b0;
            left_we  <= 1'b0;
            lb_we    <= 1'b0;
            op_valid <= 1'b0;


            // ---- the edge pipeline ----
            s1_v <= op_issue;
            if (op_issue) begin
                s1_vert   <= op_vert;
                s1_chroma <= !luma;
                s1_e      <= op_e;
                s1_r      <= op_r;
                s1_c      <= op_c;
                s1_k      <= 2'd1;
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
                s2_k    <= s1_k + {1'b0, s1_vert ? rot_v : rot_h};
                s2_out  <= {f_q2, f_q1, f_q0, f_p0, f_p1, f_p2};
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
