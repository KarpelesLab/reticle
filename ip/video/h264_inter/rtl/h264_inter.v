// h264_inter — the inter prediction samples of ITU-T H.264 clause 8.4.2,
// from reference pictures in tiled SDRAM, for `h264_recon`.
//
// What it does
//   It watches the token stream behind `h264_mvpred` (the same tokens
//   `h264_recon` gets; this port only ever takes, it never changes the
//   stream) and, for every macroblock that has `MV` tokens, sends the
//   prediction of 8.4.2.2 on `pred_*` in the contract's order: luma 16 rows
//   of 4 words, then Cb 8 rows of 2, then Cr 8 rows of 2, four samples a
//   word, the leftmost in bits 7:0. One prediction per inter macroblock, in
//   macroblock order; an intra macroblock gets none.
//
//   * 8.4.2.2.1, luma: the 6-tap filter (1, -5, 20, 20, -5, 1) for b, h, s
//     and m with their (x + 16) >> 5 rounding and clip, j from the
//     unrounded horizontal intermediates b1 filtered vertically with
//     (x + 512) >> 10, and the quarter positions as the rounded-up average
//     of the two Table 8-12 names. Every one of the sixteen positions is the
//     same datapath with a different pair of inputs to that average.
//   * 8.4.2.2.2, chroma: the eighth-sample bilinear of 8-270, done as a
//     horizontal then a vertical blend, which is the same integer sum.
//   * 8-239/8-240 and 8-262..8-269: every reference sample coordinate is
//     clamped into the picture, so a vector pointing anywhere outside it
//     reads the edge samples, as the Recommendation says.
//   * 8.4.2.1: refIdxL0 becomes a frame index through `map_frames`, a table
//     the DPB logic supplies once per slice (see "The reference map").
//   Weighted prediction (8.4.2.3.2) is not here: Baseline forbids it, and
//   8.4.2.3.1's default is the unweighted sample this computes. List 1 and
//   bi-prediction are not here either; the task loop has room for them.
//
// How a macroblock goes
//   Collector: the MV tokens are stored in one of two vector banks, by
//   block, and as each arrives it is compared with the blocks that could
//   share a partition with it. At `MB_CBP` the macroblock's sixteen
//   vectors become the coarsest set of rectangles with one vector each --
//   16x16, 16x8, 8x16, or per 8x8 one of 8x8, 8x4, 4x8, 4x4 -- which is the
//   coded partitioning or coarser. Equal vectors predict equal samples, so
//   merging them changes nothing but the work.
//
//   Each rectangle is three tasks, Y then Cb then Cr. A task is a window of
//   the reference plane: the partition, plus the filter's margin where the
//   vector has a fraction (5 samples for luma, 1 for chroma), clamped.
//
//   Check: every 8-sample line of the window is looked up in the reference
//   cache; a miss writes its tag and queues a four-word burst. Interpolate:
//   once the task's fills are in, the window is streamed one 9-sample row
//   segment a cycle through the filters, a column of four outputs at a
//   time, and the outputs land in a prediction buffer. The check of one
//   task runs while the previous one interpolates; consecutive tasks are
//   always of different planes and the planes have disjoint parts of the
//   cache, which is what makes that overlap safe.
//
//   Stream: the prediction buffer holds two macroblocks; a complete one is
//   sent while the next is computed.
//
// The reference cache
//   Direct mapped, lines of 8 samples (four SDRAM words, one burst), two
//   banks of 64-bit words so that any 9-sample segment is one read of each.
//   The index is frame[1:0], the row's low bits and the line's low bits:
//   luma 32 rows x 16 lines (128 samples wide), each chroma plane 16 rows x
//   8 lines, for four frames: 24 KB of data, a 13-bit tag a line. A line is
//   always fetched whole. It is flushed at every picture: a frame index is
//   reused for a new picture by the DPB, and a cache that outlived a
//   picture could return a sample of the old one. `README.md` has what it
//   saves on real streams, which is less than one might hope.
//
// The reference map
//   `map_frames` is sixteen 4-bit frame indices, refIdxL0 0 in bits 3:0.
//   At every `SLICE_HDR` this block stops taking tokens and raises
//   `map_ready` until `map_valid`, then copies the table. The DPB logic
//   therefore holds `map_valid` low until the table for the slice whose
//   header is waiting is on the wires.
//
// Memory
//   The sdram_burst client port of the contract, reads only. Addresses come
//   from `h264_frame_addr_stub`, a placeholder for the SDRAM agent's
//   `frame_addr` with the same ports. `rd_*` has no ready, so every word is
//   taken as it arrives.
module h264_inter #(
    parameter integer FB = 4                         // frame index bits
) (
    input  wire        clk,
    input  wire        rst_n,

    // The token stream behind h264_mvpred (a tap: this only takes).
    input  wire [31:0] tok,
    input  wire        tok_valid,
    output wire        tok_ready,

    // refIdxL0 -> frame index, per slice.
    input  wire [16*FB-1:0] map_frames,
    input  wire        map_valid,
    output wire        map_ready,

    // The sdram_burst client port.
    output wire        cmd_valid,
    input  wire        cmd_ready,
    output wire        cmd_we,
    output wire [23:0] cmd_addr,
    output wire [3:0]  cmd_len,
    output wire        wr_valid,
    input  wire        wr_ready,
    output wire [15:0] wr_data,
    output wire [1:0]  wr_be,
    input  wire        rd_valid,
    input  wire [15:0] rd_data,
    input  wire        rd_last,

    // Prediction samples, to h264_recon.
    output reg  [31:0] pred_data,
    output reg         pred_valid,
    input  wire        pred_ready
);
    localparam [7:0] T_SPS_DIMS    = 8'h01;
    localparam [7:0] T_SLICE_HDR   = 8'h02;
    localparam [7:0] T_SLICE_FIRST = 8'h06;
    localparam [7:0] T_MB_START    = 8'h10;
    localparam [7:0] T_MB_TYPE     = 8'h11;
    localparam [7:0] T_MV          = 8'h16;
    localparam [7:0] T_MB_CBP      = 8'h19;

    // Lines in the reference cache (see baddr below).
    localparam integer LINES = 3072;

    // A block's vector as stored: {frame, mvy (13), mvx (16)}.
    localparam integer VW = FB + 29;

    assign cmd_we   = 1'b0;
    assign cmd_len  = 4'd3;
    assign wr_valid = 1'b0;
    assign wr_data  = 16'd0;
    assign wr_be    = 2'b00;

    // ===============================================================
    // Collector
    // ===============================================================
    localparam [1:0] C_RUN = 2'd0, C_MAP = 2'd1, C_WAIT = 2'd2;
    reg  [1:0]  c_state;
    assign tok_ready = (c_state == C_RUN);
    assign map_ready = (c_state == C_MAP);
    wire        c_take = tok_valid && tok_ready;
    wire [7:0]  c_tag  = tok[31:24];

    reg  [6:0]  pic_w;              // PicWidthInMbs
    reg  [6:0]  pic_h;              // FrameHeightInMbs
    // The map, copied whole at the slice header.
    reg  [16*FB-1:0] rmapf;
    always @(posedge clk) begin
        if (!rst_n)                              rmapf <= {16*FB{1'b0}};
        else if (c_state == C_MAP && map_valid)  rmapf <= map_frames;
    end
    // refIdx -> frame, a shift rather than an array of wires.
    wire [16*FB-1:0] rmap_sh = rmapf >> (FB * tok[16:13]);
    reg  [6:0]  c_mbx, c_mby;
    reg         c_inter;
    reg         c_flush_pend;       // the next job starts a picture
    reg         c_bank;             // the vector bank being written
    reg  [15:0] c_mvx;              // the horizontal half of the next block

    // Two banks of sixteen vectors, by raster block.
    reg  [VW-1:0] mvb [0:31];
    reg           mvb_we;
    reg  [4:0]    mvb_wa;
    reg  [VW-1:0] mvb_wd;
    always @(posedge clk) begin
        if (mvb_we) mvb[mvb_wa] <= mvb_wd;
    end

    // The equalities the merge needs, gathered as blocks arrive in
    // luma4x4BlkIdx order. In each 8x8 (blocks 4q .. 4q+3): t = top pair,
    // u = bottom pair, l = left pair, r = right pair. Between 8x8s, each
    // compared by its first block.
    reg  [VW-1:0] prev1, prev2, first0, first1, first2;
    reg  [3:0]    e_t, e_u, e_l, e_r;
    reg           e_q01, e_q02, e_q13, e_q23;
    wire [3:0]    m_blk = tok[22:19];
    wire [VW-1:0] m_cur = {rmap_sh[FB-1:0], tok[12:0], c_mvx};
    wire          eq1 = (m_cur == prev1);
    wire          eq2 = (m_cur == prev2);

    // Per bank: the job.
    reg  [1:0]  busy;
    reg  [6:0]  j_x   [0:1];
    reg  [6:0]  j_y   [0:1];
    reg  [11:0] j_shape [0:1];      // {mode[1:0], quadrant shapes 4 x 2}
    reg         j_flush [0:1];
    reg         g_release;          // the generator has finished a bank...
    reg         g_rbank;            // ...this one
    reg         g_bank;

    wire [3:0] uni = e_t & e_u & e_l;
    wire       all_uni = &uni;
    wire [1:0] mode = (all_uni && e_q01 && e_q02 && e_q13) ? 2'd0 :
                      (all_uni && e_q01 && e_q23)          ? 2'd1 :
                      (all_uni && e_q02 && e_q13)          ? 2'd2 : 2'd3;
    function [1:0] qshape;
        input u, t, b, l, r;
        begin
            if (u)           qshape = 2'd0;   // 8x8
            else if (t && b) qshape = 2'd1;   // 8x4
            else if (l && r) qshape = 2'd2;   // 4x8
            else             qshape = 2'd3;   // 4x4
        end
    endfunction
    wire [1:0] qs0 = qshape(uni[0], e_t[0], e_u[0], e_l[0], e_r[0]);
    wire [1:0] qs1 = qshape(uni[1], e_t[1], e_u[1], e_l[1], e_r[1]);
    wire [1:0] qs2 = qshape(uni[2], e_t[2], e_u[2], e_l[2], e_r[2]);
    wire [1:0] qs3 = qshape(uni[3], e_t[3], e_u[3], e_l[3], e_r[3]);

    integer k;
    always @(posedge clk) begin
        if (!rst_n) begin
            c_state      <= C_RUN;
            pic_w        <= 7'd0;
            pic_h        <= 7'd0;
            c_mbx        <= 7'd0;
            c_mby        <= 7'd0;
            c_inter      <= 1'b0;
            c_flush_pend <= 1'b1;
            c_bank       <= 1'b0;
            c_mvx        <= 16'd0;
            mvb_we       <= 1'b0;
            mvb_wa       <= 5'd0;
            mvb_wd       <= {VW{1'b0}};
            prev1        <= {VW{1'b0}};
            prev2        <= {VW{1'b0}};
            first0       <= {VW{1'b0}};
            first1       <= {VW{1'b0}};
            first2       <= {VW{1'b0}};
            e_t <= 4'd0; e_u <= 4'd0; e_l <= 4'd0; e_r <= 4'd0;
            e_q01 <= 1'b0; e_q02 <= 1'b0; e_q13 <= 1'b0; e_q23 <= 1'b0;
            busy         <= 2'b00;
            for (k = 0; k < 2; k = k + 1) begin
                j_x[k] <= 7'd0; j_y[k] <= 7'd0; j_shape[k] <= 12'd0; j_flush[k] <= 1'b0;
            end
        end else begin
            mvb_we <= 1'b0;
            if (g_release) busy[g_rbank] <= 1'b0;
            case (c_state)
            C_RUN: if (c_take) begin
                case (c_tag)
                    T_SPS_DIMS: begin
                        pic_w <= tok[18:12];
                        pic_h <= tok[6:0];
                    end
                    T_SLICE_HDR:   c_state <= C_MAP;
                    T_SLICE_FIRST: if (tok[23:0] == 24'd0) c_flush_pend <= 1'b1;
                    T_MB_START: begin
                        c_mbx <= tok[18:12];
                        c_mby <= tok[6:0];
                    end
                    T_MB_TYPE: c_inter <= 1'b0;
                    T_MV: if (!tok[23]) begin
                        if (!tok[18]) begin
                            c_mvx <= tok[15:0];
                        end else begin
                            c_inter <= 1'b1;
                            mvb_we  <= 1'b1;
                            mvb_wa  <= {c_bank, m_blk[3], m_blk[1], m_blk[2], m_blk[0]};
                            mvb_wd  <= m_cur;
                            prev2   <= prev1;
                            prev1   <= m_cur;
                            if (m_blk[0])        // second of a pair along a row
                                if (m_blk[1]) e_u[m_blk[3:2]] <= eq1;
                                else          e_t[m_blk[3:2]] <= eq1;
                            if (m_blk[1])        // second row of the 8x8
                                if (m_blk[0]) e_r[m_blk[3:2]] <= eq2;
                                else          e_l[m_blk[3:2]] <= eq2;
                            case (m_blk)
                                4'd0:  first0 <= m_cur;
                                4'd4: begin first1 <= m_cur; e_q01 <= (m_cur == first0); end
                                4'd8: begin first2 <= m_cur; e_q02 <= (m_cur == first0); end
                                4'd12: begin
                                    e_q13 <= (m_cur == first1);
                                    e_q23 <= (m_cur == first2);
                                end
                                default: ;
                            endcase
                        end
                    end
                    T_MB_CBP: if (c_inter) begin
                        j_x[c_bank]     <= c_mbx;
                        j_y[c_bank]     <= c_mby;
                        j_shape[c_bank] <= {mode, qs0, qs1, qs2, qs3, 2'b00};
                        j_flush[c_bank] <= c_flush_pend;
                        c_flush_pend    <= 1'b0;
                        busy[c_bank]    <= 1'b1;
                        c_bank          <= !c_bank;
                        c_inter         <= 1'b0;
                        c_state         <= C_WAIT;
                    end
                    default: ;
                endcase
            end
            C_MAP: if (map_valid) c_state <= C_RUN;
            // The bank the next macroblock's vectors go into must be free.
            C_WAIT: if (!busy[c_bank] || (g_release && g_rbank == c_bank)) c_state <= C_RUN;
            default: c_state <= C_RUN;
            endcase
        end
    end

    // ===============================================================
    // Task generator
    // ===============================================================
    // Partition walk: for mode 3, quadrant g_q and sub-block g_s by the
    // quadrant's shape; otherwise one or two rectangles.
    reg  [1:0]  g_q, g_s;
    reg  [1:0]  g_plane;
    reg         g_run;              // a bank is being walked
    reg         g_flushed;          // this bank's flush is done
    reg         g_flushing;
    reg  [11:0] g_fl;               // flush walk position
    reg  [1:0]  pb_full;            // prediction buffer bank waits to be sent
    reg         pb_wbank;           // the bank this macroblock fills

    wire [11:0] g_shape = j_shape[g_bank];
    wire [1:0]  g_mode  = g_shape[11:10];
    reg  [1:0]  g_qsh;
    always @* begin
        case (g_q)
            2'd0: g_qsh = g_shape[9:8];
            2'd1: g_qsh = g_shape[7:6];
            2'd2: g_qsh = g_shape[5:4];
            default: g_qsh = g_shape[3:2];
        endcase
    end

    // The current rectangle, in 4x4 blocks, and whether it is the last.
    reg  [1:0] p_bx, p_by;
    reg  [2:0] p_bw, p_bh;
    reg        p_last;
    always @* begin
        p_bx = 2'd0; p_by = 2'd0; p_bw = 3'd4; p_bh = 3'd4; p_last = 1'b1;
        case (g_mode)
            2'd0: ;
            2'd1: begin p_by = {g_s[0], 1'b0}; p_bh = 3'd2; p_last = g_s[0]; end
            2'd2: begin p_bx = {g_s[0], 1'b0}; p_bw = 3'd2; p_last = g_s[0]; end
            default: begin
                p_bx = {g_q[0], 1'b0};
                p_by = {g_q[1], 1'b0};
                case (g_qsh)
                    2'd0: begin p_bw = 3'd2; p_bh = 3'd2; p_last = (g_q == 2'd3); end
                    2'd1: begin p_bw = 3'd2; p_bh = 3'd1; p_by = {g_q[1], g_s[0]};
                                p_last = (g_q == 2'd3) && g_s[0]; end
                    2'd2: begin p_bw = 3'd1; p_bh = 3'd2; p_bx = {g_q[0], g_s[0]};
                                p_last = (g_q == 2'd3) && g_s[0]; end
                    default: begin
                        p_bw = 3'd1; p_bh = 3'd1;
                        p_bx = {g_q[0], g_s[0]};
                        p_by = {g_q[1], g_s[1]};
                        p_last = (g_q == 2'd3) && (g_s == 2'd3);
                    end
                endcase
            end
        endcase
    end
    // Is this the last sub-rectangle of its 8x8 (mode 3)?
    wire q_done = (g_qsh == 2'd0) || ((g_qsh != 2'd3) && g_s[0]) || (g_s == 2'd3);

    wire [VW-1:0] g_mv = mvb[{g_bank, p_by, p_bx}];
    wire [FB-1:0] g_frame = g_mv[VW-1:29];
    wire signed [15:0] g_mvy = {{3{g_mv[28]}}, g_mv[28:16]};
    wire signed [15:0] g_mvx = g_mv[15:0];

    // The window of the task, before clamping, in samples of the plane.
    wire        g_luma = (g_plane == 2'd0);
    wire [2:0]  g_xf = g_luma ? {1'b0, g_mvx[1:0]} : g_mvx[2:0];
    wire [2:0]  g_yf = g_luma ? {1'b0, g_mvy[1:0]} : g_mvy[2:0];
    wire [6:0]  g_mbx = j_x[g_bank];
    wire [6:0]  g_mby = j_y[g_bank];
    wire signed [14:0] g_xi = g_luma
        ? $signed({4'd0, g_mbx, p_bx, 2'b00}) + (g_mvx >>> 2)
        : $signed({5'd0, g_mbx, p_bx, 1'b0}) + (g_mvx >>> 3);
    wire signed [14:0] g_yi = g_luma
        ? $signed({4'd0, g_mby, p_by, 2'b00}) + (g_mvy >>> 2)
        : $signed({5'd0, g_mby, p_by, 1'b0}) + (g_mvy >>> 3);
    wire [4:0]  g_pw = g_luma ? {p_bw, 2'b00} : {1'b0, p_bw, 1'b0};
    wire [4:0]  g_ph = g_luma ? {p_bh, 2'b00} : {1'b0, p_bh, 1'b0};
    wire        g_xfn = (g_xf != 3'd0);
    wire        g_yfn = (g_yf != 3'd0);
    // A segment starts two samples left of the integer column for luma,
    // so that the 6-tap filter's taps are at fixed places in it.
    wire signed [14:0] g_sx0 = g_luma ? g_xi - 15'sd2 : g_xi;
    wire signed [14:0] g_ry0 = (g_luma && g_yfn) ? g_yi - 15'sd2 : g_yi;
    wire [4:0]  g_rows = g_ph + (g_yfn ? (g_luma ? 5'd5 : 5'd1) : 5'd0);
    // The columns the filter reads.
    wire signed [14:0] g_fc0 = (g_luma && g_xfn) ? g_xi - 15'sd2 : g_xi;
    wire signed [14:0] g_fc1 = g_xi + $signed({10'd0, g_pw}) - 15'sd1 +
                               (g_xfn ? (g_luma ? 15'sd3 : 15'sd1) : 15'sd0);
    wire signed [14:0] g_fr1 = g_ry0 + $signed({10'd0, g_rows}) - 15'sd1;

    wire [10:0] g_wmax = g_luma ? {pic_w, 4'd0} - 11'd1 : {1'b0, pic_w, 3'd0} - 11'd1;
    wire [10:0] g_hmax = g_luma ? {pic_h, 4'd0} - 11'd1 : {1'b0, pic_h, 3'd0} - 11'd1;

    function [10:0] clampv;
        input signed [14:0] v;
        input [10:0] hi;
        begin
            if (v < 15'sd0)                    clampv = 11'd0;
            else if (v > $signed({4'd0, hi}))  clampv = hi;
            else                               clampv = v[10:0];
        end
    endfunction
    wire [10:0] g_c0 = clampv(g_fc0, g_wmax);
    wire [10:0] g_c1 = clampv(g_fc1, g_wmax);
    wire [10:0] g_r0 = clampv(g_ry0, g_hmax);
    wire [10:0] g_r1 = clampv(g_fr1, g_hmax);

    // The task register between the generator and the check.
    reg         t_valid;
    reg  [1:0]  t_plane;
    reg  [FB-1:0] t_frame;
    reg         t_pbank;
    reg         t_last;             // the macroblock's last task
    reg  [1:0]  t_bx, t_by;
    reg  [2:0]  t_bw, t_bh;
    reg  [2:0]  t_xf, t_yf;
    reg  signed [14:0] t_sx0, t_ry0;
    reg  [4:0]  t_rows;
    reg  [10:0] t_c0, t_c1, t_r0, t_r1;
    reg  [10:0] t_wmax, t_hmax;
    wire        t_take;

    wire        k_idle;
    wire        i_idle;
    wire        fills_done;
    wire        pb_done;            // the streamer finished a bank
    wire        pb_done_bank;
    wire        pb_ready_set;       // the interpolator finished a bank
    wire        pb_ready_bank;

    always @(posedge clk) begin
        if (!rst_n) begin
            g_q <= 2'd0; g_s <= 2'd0; g_plane <= 2'd0;
            g_run <= 1'b0; g_bank <= 1'b0; g_release <= 1'b0;
            g_flushed <= 1'b0; g_flushing <= 1'b0; g_fl <= 12'd0; g_rbank <= 1'b0;
            pb_wbank <= 1'b0;
            pb_full <= 2'b00;
            t_valid <= 1'b0; t_plane <= 2'd0; t_frame <= {FB{1'b0}};
            t_pbank <= 1'b0; t_last <= 1'b0;
            t_bx <= 2'd0; t_by <= 2'd0; t_bw <= 3'd0; t_bh <= 3'd0;
            t_xf <= 3'd0; t_yf <= 3'd0;
            t_sx0 <= 15'sd0; t_ry0 <= 15'sd0; t_rows <= 5'd0;
            t_c0 <= 11'd0; t_c1 <= 11'd0; t_r0 <= 11'd0; t_r1 <= 11'd0;
            t_wmax <= 11'd0; t_hmax <= 11'd0;
        end else begin
            g_release <= 1'b0;
            if (t_take) t_valid <= 1'b0;
            // A prediction-buffer bank is claimed when its macroblock
            // starts and freed when the streamer has sent it.
            if (pb_done) pb_full[pb_done_bank] <= 1'b0;
            if (g_flushing) begin
                g_fl <= g_fl + 12'd1;
                if (g_fl == LINES - 1) begin
                    g_flushing <= 1'b0;
                    g_flushed  <= 1'b1;
                end
            end else if (!g_run) begin
                if (busy[g_bank] && !g_release && !pb_full[pb_wbank]) begin
                    if (j_flush[g_bank] && !g_flushed) begin
                        if (!t_valid && k_idle && i_idle && fills_done) begin
                            g_flushing <= 1'b1;
                            g_fl       <= 12'd0;
                        end
                    end else begin
                        g_run <= 1'b1;
                        pb_full[pb_wbank] <= 1'b1;
                        g_q <= 2'd0; g_s <= 2'd0; g_plane <= 2'd0;
                    end
                end
            end else if (!t_valid || t_take) begin
                t_valid <= 1'b1;
                t_plane <= g_plane;
                t_frame <= g_frame;
                t_pbank <= pb_wbank;
                t_last  <= p_last && (g_plane == 2'd2);
                t_bx <= p_bx; t_by <= p_by; t_bw <= p_bw; t_bh <= p_bh;
                t_xf <= g_xf; t_yf <= g_yf;
                t_sx0 <= g_sx0; t_ry0 <= g_ry0; t_rows <= g_rows;
                t_c0 <= g_c0; t_c1 <= g_c1; t_r0 <= g_r0; t_r1 <= g_r1;
                t_wmax <= g_wmax; t_hmax <= g_hmax;
                if (g_plane != 2'd2) begin
                    g_plane <= g_plane + 2'd1;
                end else begin
                    g_plane <= 2'd0;
                    if (p_last) begin
                        g_run     <= 1'b0;
                        g_release <= 1'b1;
                        g_rbank   <= g_bank;
                        g_flushed <= 1'b0;
                        g_bank    <= !g_bank;
                        pb_wbank  <= !pb_wbank;
                    end else if (g_mode != 2'd3 || !q_done) begin
                        g_s <= g_s + 2'd1;
                    end else begin
                        g_q <= g_q + 2'd1;
                        g_s <= 2'd0;
                    end
                end
            end
        end
    end

    // ===============================================================
    // The reference cache: index, tags, data
    // ===============================================================
    // Bank address (11 bits) of line `xl` (eight samples) of row `y`, and
    // the tag that tells that line from the others sharing its place. The
    // bank is xl[0]; the tag RAM is indexed by {bank address, bank}. Luma
    // takes bank addresses 0..1023, Cb 1024..1279 and Cr 1280..1535.
    function [10:0] baddr;
        input [1:0]    plane;
        input [FB-1:0] frame;
        input [10:0]   y;
        input [7:0]    xl;
        begin
            if (plane == 2'd0) baddr = {1'b0, frame[1:0], y[4:0], xl[3:1]};
            else               baddr = {2'b10, plane[1], frame[1:0], y[3:0], xl[2:1]};
        end
    endfunction
    function [12:0] tagof;          // {valid, frame[3:2], row high (6), line high (4)}
        input [1:0]    plane;
        input [FB-1:0] frame;
        input [10:0]   y;
        input [7:0]    xl;
        begin
            if (plane == 2'd0) tagof = {1'b1, frame[FB-1:2], y[10:5], xl[7:4]};
            else               tagof = {1'b1, frame[FB-1:2], y[9:4], xl[6:3]};
        end
    endfunction

    reg  [12:0] tagram [0:LINES-1];
    reg  [12:0] tag_q;
    wire [11:0] tag_ra;
    reg         tag_we;
    reg  [11:0] tag_wa;
    reg  [12:0] tag_wd;
    always @(posedge clk) begin
        if (tag_we) tagram[tag_wa] <= tag_wd;
        tag_q <= tagram[tag_ra];
    end

    reg  [63:0] dram0 [0:LINES/2-1];    // even lines
    reg  [63:0] dram1 [0:LINES/2-1];    // odd lines
    reg  [63:0] d0_q, d1_q;
    reg  [10:0] d0_ra, d1_ra;
    reg         d_we;
    reg         d_wbank;
    reg  [10:0] d_wa;
    reg  [63:0] d_wd;
    always @(posedge clk) begin
        if (d_we && !d_wbank) dram0[d_wa] <= d_wd;
        if (d_we &&  d_wbank) dram1[d_wa] <= d_wd;
        d0_q <= dram0[d0_ra];
        d1_q <= dram1[d1_ra];
    end

    // ===============================================================
    // Check: look up every line of the window
    // ===============================================================
    reg         k_busy;
    reg  [1:0]  k_plane;
    reg  [FB-1:0] k_frame;
    reg  [10:0] k_y, k_r1;
    reg  [7:0]  k_xl, k_l0, k_l1;
    reg         k_done;             // the last lookup has been issued
    // The lookup in flight: issued last cycle, compared this one.
    reg         k_v;
    reg  [12:0] k_tag;
    reg  [11:0] k_idx;
    reg  [10:0] k_vy;
    reg  [7:0]  k_vxl;
    // The rest of the task, for the interpolator.
    reg         kt_pbank, kt_last;
    reg  [1:0]  kt_bx, kt_by;
    reg  [2:0]  kt_bw, kt_bh, kt_xf, kt_yf;
    reg  signed [14:0] kt_sx0, kt_ry0;
    reg  [4:0]  kt_rows;
    reg  [10:0] kt_wmax, kt_hmax;

    // Misses waiting for the command port: {frame, plane, y, xl, index}.
    localparam integer RQW = FB + 2 + 11 + 8 + 12;
    reg  [RQW-1:0] rq [0:15];
    reg  [4:0]  rq_n;
    reg  [3:0]  rq_rd, rq_wr;
    reg  [5:0]  req_seq, fill_seq;

    wire        k_room  = (rq_n <= 5'd13);
    wire        k_issue = k_busy && !k_done && k_room;
    assign      t_take  = t_valid && !k_busy;
    assign      k_idle  = !k_busy && !k_v;
    wire        k_miss  = k_v && (tag_q != k_tag);
    assign      tag_ra  = {baddr(k_plane, k_frame, k_y, k_xl), k_xl[0]};

    reg         i_busy;
    reg         i_wait;             // waiting for its fills
    wire        k_hand = k_busy && k_done && !k_v && !i_busy;

    always @(posedge clk) begin
        if (!rst_n) begin
            k_busy <= 1'b0; k_done <= 1'b0; k_v <= 1'b0;
            k_plane <= 2'd0; k_frame <= {FB{1'b0}};
            k_y <= 11'd0; k_r1 <= 11'd0; k_xl <= 8'd0; k_l0 <= 8'd0; k_l1 <= 8'd0;
            k_tag <= 13'd0; k_idx <= 12'd0; k_vy <= 11'd0; k_vxl <= 8'd0;
            kt_pbank <= 1'b0; kt_last <= 1'b0; kt_bx <= 2'd0; kt_by <= 2'd0;
            kt_bw <= 3'd0; kt_bh <= 3'd0; kt_xf <= 3'd0; kt_yf <= 3'd0;
            kt_sx0 <= 15'sd0; kt_ry0 <= 15'sd0; kt_rows <= 5'd0;
            kt_wmax <= 11'd0; kt_hmax <= 11'd0;
        end else begin
            if (t_take) begin
                k_busy  <= 1'b1;
                k_done  <= 1'b0;
                k_plane <= t_plane; k_frame <= t_frame;
                k_y <= t_r0; k_r1 <= t_r1;
                k_xl <= t_c0[10:3]; k_l0 <= t_c0[10:3]; k_l1 <= t_c1[10:3];
                kt_pbank <= t_pbank; kt_last <= t_last;
                kt_bx <= t_bx; kt_by <= t_by; kt_bw <= t_bw; kt_bh <= t_bh;
                kt_xf <= t_xf; kt_yf <= t_yf;
                kt_sx0 <= t_sx0; kt_ry0 <= t_ry0; kt_rows <= t_rows;
                kt_wmax <= t_wmax; kt_hmax <= t_hmax;
            end
            k_v <= k_issue;
            if (k_issue) begin
                k_idx  <= tag_ra;
                k_tag  <= tagof(k_plane, k_frame, k_y, k_xl);
                k_vy   <= k_y;
                k_vxl  <= k_xl;
                if (k_xl == k_l1) begin
                    k_xl <= k_l0;
                    if (k_y == k_r1) k_done <= 1'b1;
                    else             k_y <= k_y + 11'd1;
                end else begin
                    k_xl <= k_xl + 8'd1;
                end
            end
            if (k_hand) k_busy <= 1'b0;
        end
    end

    // Tag writes: a miss, or the flush walk.
    always @* begin
        tag_we = 1'b0;
        tag_wa = k_idx;
        tag_wd = k_tag;
        if (g_flushing) begin
            tag_we = 1'b1;
            tag_wa = g_fl;
            tag_wd = 13'd0;
        end else if (k_miss) begin
            tag_we = 1'b1;
        end
    end

    // ===============================================================
    // Fetch: commands out, data in
    // ===============================================================
    wire [RQW-1:0] rq_head = rq[rq_rd];
    wire [11:0] h_idx   = rq_head[11:0];
    wire [7:0]  h_xl    = rq_head[19:12];
    wire [10:0] h_y     = rq_head[30:20];
    wire [1:0]  h_plane = rq_head[32:31];
    wire [FB-1:0] h_frame = rq_head[RQW-1:33];

    // Lines whose command has gone and whose data has not all come back.
    reg  [11:0] pend [0:15];
    reg  [4:0]  pend_n;
    reg  [3:0]  pend_rd, pend_wr;

    assign cmd_valid = (rq_n != 5'd0) && (pend_n != 5'd16);
    wire   cmd_go    = cmd_valid && cmd_ready;
    h264_frame_addr_stub #(.FB(FB)) u_addr (
        .frame(h_frame),
        .plane(h_plane),
        .x({h_xl, 3'b000}),
        .y(h_y),
        .addr(cmd_addr)
    );

    reg  [47:0] f_acc;
    reg  [1:0]  f_n;
    wire        f_done = rd_valid && (f_n == 2'd3);
    wire [11:0] f_idx  = pend[pend_rd];
    assign      fills_done = (fill_seq == req_seq);

    always @(posedge clk) begin
        if (k_miss) rq[rq_wr] <= {k_frame, k_plane, k_vy, k_vxl, k_idx};
        if (cmd_go) pend[pend_wr] <= h_idx;
    end

    always @(posedge clk) begin
        if (!rst_n) begin
            rq_n <= 5'd0; rq_rd <= 4'd0; rq_wr <= 4'd0;
            pend_n <= 5'd0; pend_rd <= 4'd0; pend_wr <= 4'd0;
            req_seq <= 6'd0; fill_seq <= 6'd0;
            f_acc <= 48'd0; f_n <= 2'd0;
            d_we <= 1'b0; d_wbank <= 1'b0; d_wa <= 11'd0; d_wd <= 64'd0;
        end else begin
            if (k_miss) begin
                rq_wr   <= rq_wr + 4'd1;
                req_seq <= req_seq + 6'd1;
            end
            if (cmd_go) begin
                rq_rd   <= rq_rd + 4'd1;
                pend_wr <= pend_wr + 4'd1;
            end
            rq_n   <= rq_n + {4'd0, k_miss} - {4'd0, cmd_go};
            pend_n <= pend_n + {4'd0, cmd_go} - {4'd0, f_done};
            d_we <= 1'b0;
            if (rd_valid) begin
                f_n   <= f_n + 2'd1;
                f_acc <= {rd_data, f_acc[47:16]};
                if (f_done) begin
                    d_we    <= 1'b1;
                    d_wbank <= f_idx[0];
                    d_wa    <= f_idx[11:1];
                    d_wd    <= {rd_data, f_acc};
                    pend_rd <= pend_rd + 4'd1;
                end
            end
            // A line counts as filled once its data RAM write is done.
            if (d_we) fill_seq <= fill_seq + 6'd1;
        end
    end

    // ===============================================================
    // Interpolate: the task loop and the read of the window
    // ===============================================================
    reg  [5:0]  i_need;
    reg  [1:0]  i_plane;
    reg  [FB-1:0] i_frame;
    reg         i_pbank, i_last;
    reg  [1:0]  i_bx, i_by;
    reg  [2:0]  i_bw, i_bh, i_xf, i_yf;
    reg  signed [14:0] i_sx0, i_ry0;
    reg  [4:0]  i_rows;
    reg  [10:0] i_wmax, i_hmax;
    reg  [2:0]  i_g;                // column group
    reg  [4:0]  i_r;                // window row
    assign      i_idle = !i_busy;

    // Column groups of four outputs; a chroma partition two wide is one
    // group of two.
    wire [2:0]  i_ng = (i_plane == 2'd0) ? i_bw : ((i_bw == 3'd1) ? 3'd1 : {1'b0, i_bw[2:1]});
    wire        i_step = i_busy && !i_wait;
    wire        i_endg = (i_r == i_rows - 5'd1);
    wire        i_end  = i_endg && (i_g == i_ng - 3'd1);

    // Stage 0: the segment's row and columns, clamped (8-239, 8-240,
    // 8-262 .. 8-269), and the two lines that hold it.
    wire signed [14:0] s_sx = i_sx0 + $signed({10'd0, i_g, 2'b00});
    wire signed [14:0] s_ry = i_ry0 + $signed({10'd0, i_r});
    wire [10:0] s_y  = clampv(s_ry, i_hmax);
    wire [10:0] s_c0 = clampv(s_sx, i_wmax);
    wire [7:0]  s_k  = s_c0[10:3];
    wire [7:0]  s_k1 = s_k + 8'd1;
    wire [7:0]  s_even = s_k[0] ? s_k1 : s_k;
    wire [7:0]  s_odd  = s_k[0] ? s_k  : s_k1;
    // Nine byte indices into the sixteen bytes of lines k and k+1: the
    // clamped column less 8k, which is never outside 0..15 because every
    // clamped column of the segment is at least clamp(sx) and at most
    // clamp(sx) + 8.
    wire [35:0] s_bi;
    genvar gb;
    generate
        for (gb = 0; gb < 9; gb = gb + 1) begin : g_bi
            wire [10:0] cl = clampv(s_sx + gb, i_wmax);
            assign s_bi[4*gb +: 4] = cl[3:0] - {s_k[0], 3'b000};
        end
    endgenerate
    // Does this row produce an output row, and which?
    wire        s_yfn  = (i_yf != 3'd0);
    wire [4:0]  s_skip = s_yfn ? ((i_plane == 2'd0) ? 5'd5 : 5'd1) : 5'd0;
    wire        s_emit = (i_r >= s_skip);
    wire [4:0]  s_orow = i_r - s_skip;
    // Where the outputs go in the 96-word layout.
    reg  [6:0]  s_word;
    reg  [3:0]  s_lanes;
    reg  [1:0]  s_shift;            // lane of the first output
    always @* begin
        s_lanes = 4'b1111;
        s_shift = 2'd0;
        if (i_plane == 2'd0) begin
            s_word = {1'b0, i_by, 4'b0000} + {1'b0, s_orow[3:0], 2'b00} + {5'd0, i_bx} + {4'd0, i_g};
        end else begin
            s_word = (i_plane[1] ? 7'd80 : 7'd64) + {3'd0, i_by, 2'b00} + {2'd0, s_orow[2:0], 1'b0}
                   + {6'd0, i_bx[1]} + {4'd0, i_g};
            if (i_bw == 3'd1) begin
                s_lanes = i_bx[0] ? 4'b1100 : 4'b0011;
                s_shift = {i_bx[0], 1'b0};
            end
        end
    end

    // The control that travels with a segment: {valid, emit, luma, xf, yf,
    // bank, word, lanes, shift, k odd, last}.
    localparam integer CW = 1 + 1 + 1 + 3 + 3 + 1 + 7 + 4 + 2 + 1 + 1;
    localparam integer F_V = 24, F_EMIT = 23, F_LUMA = 22, F_XF = 19, F_YF = 16,
                       F_BANK = 15, F_WORD = 8, F_LANES = 4, F_SHIFT = 2,
                       F_KODD = 1, F_LAST = 0;
    wire [CW-1:0] c0 = {i_step, s_emit, (i_plane == 2'd0), i_xf, i_yf, i_pbank,
                        s_word, s_lanes, s_shift, s_k[0], i_step && i_end && i_last};
    reg  [CW-1:0] c1, c2, c3, c4, c5, c6;
    reg  [35:0]   bi1, bi2;
    reg  [71:0]   seg;              // nine samples, the first in bits 7:0
    wire [127:0]  pair = c2[F_KODD] ? {d0_q, d1_q} : {d1_q, d0_q};
    wire [71:0]   seg_n;
    genvar gs;
    generate
        for (gs = 0; gs < 9; gs = gs + 1) begin : g_seg
            wire [127:0] sh = pair >> {bi2[4*gs +: 4], 3'b000};
            assign seg_n[8*gs +: 8] = sh[7:0];
        end
    endgenerate

    always @(posedge clk) begin
        if (!rst_n) begin
            c1 <= {CW{1'b0}}; c2 <= {CW{1'b0}}; c3 <= {CW{1'b0}};
            bi1 <= 36'd0; bi2 <= 36'd0; seg <= 72'd0;
            d0_ra <= 11'd0; d1_ra <= 11'd0;
        end else begin
            // Stage 1: the data RAM reads its two lines.
            c1    <= c0;
            bi1   <= s_bi;
            d0_ra <= baddr(i_plane, i_frame, s_y, s_even);
            d1_ra <= baddr(i_plane, i_frame, s_y, s_odd);
            // Stage 2: the lines are out; stage 3: the segment.
            c2  <= c1;
            bi2 <= bi1;
            c3  <= c2;
            seg <= seg_n;
        end
    end

    // The interpolator's task loop.
    always @(posedge clk) begin
        if (!rst_n) begin
            i_busy <= 1'b0; i_wait <= 1'b0; i_need <= 6'd0;
            i_plane <= 2'd0; i_frame <= {FB{1'b0}}; i_pbank <= 1'b0; i_last <= 1'b0;
            i_bx <= 2'd0; i_by <= 2'd0; i_bw <= 3'd0; i_bh <= 3'd0;
            i_xf <= 3'd0; i_yf <= 3'd0; i_sx0 <= 15'sd0; i_ry0 <= 15'sd0;
            i_rows <= 5'd0; i_wmax <= 11'd0; i_hmax <= 11'd0;
            i_g <= 3'd0; i_r <= 5'd0;
        end else begin
            if (k_hand) begin
                i_busy  <= 1'b1;
                i_wait  <= 1'b1;
                i_need  <= req_seq;
                i_plane <= k_plane; i_frame <= k_frame;
                i_pbank <= kt_pbank; i_last <= kt_last;
                i_bx <= kt_bx; i_by <= kt_by; i_bw <= kt_bw; i_bh <= kt_bh;
                i_xf <= kt_xf; i_yf <= kt_yf; i_sx0 <= kt_sx0; i_ry0 <= kt_ry0;
                i_rows <= kt_rows; i_wmax <= kt_wmax; i_hmax <= kt_hmax;
                i_g <= 3'd0; i_r <= 5'd0;
            end else if (i_busy && i_wait) begin
                if (fill_seq == i_need) i_wait <= 1'b0;
            end else if (i_step) begin
                if (i_endg) begin
                    i_r <= 5'd0;
                    if (i_end) i_busy <= 1'b0;
                    else       i_g <= i_g + 3'd1;
                end else begin
                    i_r <= i_r + 5'd1;
                end
            end
        end
    end

    // ===============================================================
    // Luma, 8.4.2.2.1
    // ===============================================================
    // 20 * x and 5 * x as shifts, so that no multiplier is inferred.
    function signed [15:0] tap6;     // E - 5F + 20G + 20H - 5I + J, integer samples
        input [7:0] e, f, g, h, i, j;
        reg   signed [15:0] gh, fi;
        begin
            gh   = $signed({8'd0, g}) + $signed({8'd0, h});
            fi   = $signed({8'd0, f}) + $signed({8'd0, i});
            tap6 = $signed({8'd0, e}) + $signed({8'd0, j}) + (gh <<< 4) + (gh <<< 2)
                 - (fi <<< 2) - fi;
        end
    endfunction
    function signed [21:0] tap6w;    // the same over the intermediates b1
        input signed [15:0] e, f, g, h, i, j;
        reg   signed [21:0] gh, fi;
        begin
            gh    = {{6{g[15]}}, g} + {{6{h[15]}}, h};
            fi    = {{6{f[15]}}, f} + {{6{i[15]}}, i};
            tap6w = {{6{e[15]}}, e} + {{6{j[15]}}, j} + (gh <<< 4) + (gh <<< 2)
                  - (fi <<< 2) - fi;
        end
    endfunction
    function [7:0] clip5;            // Clip1Y((x + 16) >> 5), 8-243
        input signed [15:0] x;
        reg   signed [15:0] t;
        begin
            t = (x + 16'sd16) >>> 5;
            if (t < 16'sd0)        clip5 = 8'd0;
            else if (t > 16'sd255) clip5 = 8'd255;
            else                   clip5 = t[7:0];
        end
    endfunction
    function [7:0] clip10;           // Clip1Y((x + 512) >> 10), 8-247
        input signed [21:0] x;
        reg   signed [21:0] t;
        begin
            t = (x + 22'sd512) >>> 10;
            if (t < 22'sd0)        clip10 = 8'd0;
            else if (t > 22'sd255) clip10 = 8'd255;
            else                   clip10 = t[7:0];
        end
    endfunction

    // Pushed with each segment (stage 3 -> 4): the integer samples of
    // columns x .. x+4 (segment bytes 2..6) and b1 for the four outputs.
    // Six rows deep, oldest first.
    reg  [39:0] lr0, lr1, lr2, lr3, lr4, lr5;
    reg  [63:0] lb0, lb1, lb2, lb3, lb4, lb5;
    wire [63:0] b1n;
    genvar go;
    generate
        for (go = 0; go < 4; go = go + 1) begin : g_b1
            assign b1n[16*go +: 16] = tap6(seg[8*go +: 8], seg[8*go+8 +: 8], seg[8*go+16 +: 8],
                                           seg[8*go+24 +: 8], seg[8*go+32 +: 8], seg[8*go+40 +: 8]);
        end
    endgenerate

    // Chroma, 8.4.2.2.2, pushed the same way: (8 - xF) A + xF B for the
    // four outputs, each 11 bits, two rows deep.
    function [13:0] mul3;            // x * f for f in 0..7, as shifts
        input [10:0] x;
        input [2:0]  f;
        begin
            mul3 = (f[0] ? {3'd0, x} : 14'd0) + (f[1] ? {2'd0, x, 1'b0} : 14'd0)
                 + (f[2] ? {1'd0, x, 2'b00} : 14'd0);
        end
    endfunction
    reg  [43:0] cr0, cr1;
    wire [43:0] hcn;
    wire [2:0]  c3_xf = c3[F_XF +: 3];
    genvar gc;
    generate
        for (gc = 0; gc < 4; gc = gc + 1) begin : g_hc
            wire [7:0]  a  = seg[8*gc +: 8];
            wire [7:0]  b  = seg[8*gc+8 +: 8];
            wire [13:0] wa = mul3({3'd0, a}, 3'd0 - c3_xf);
            wire [13:0] wb = mul3({3'd0, b}, c3_xf);
            wire [13:0] sum = wa + wb;
            assign hcn[11*gc +: 11] = (c3_xf == 3'd0) ? {a, 3'b000} : sum[10:0];
        end
    endgenerate

    always @(posedge clk) begin
        if (c3[F_V]) begin
            lr0 <= lr1; lr1 <= lr2; lr2 <= lr3; lr3 <= lr4; lr4 <= lr5;
            lr5 <= seg[55:16];
            lb0 <= lb1; lb1 <= lb2; lb2 <= lb3; lb3 <= lb4; lb4 <= lb5;
            lb5 <= b1n;
            cr0 <= cr1;
            cr1 <= hcn;
        end
    end

    // Stage 4: the named samples of Figure 8-4 for each output, clipped.
    // With yFrac 0 the current row is the newest; otherwise it is the
    // third of the six, and the window's first two rows were y-2, y-1.
    wire        c4_yfn = (c4[F_YF +: 3] != 3'd0);
    wire [31:0] n_G, n_H, n_M, n_b, n_s, n_h, n_m, n_j;
    wire [39:0] hv;
    genvar gh, gq;
    generate
        for (gh = 0; gh < 5; gh = gh + 1) begin : g_hv
            assign hv[8*gh +: 8] = clip5(tap6(lr0[8*gh +: 8], lr1[8*gh +: 8], lr2[8*gh +: 8],
                                              lr3[8*gh +: 8], lr4[8*gh +: 8], lr5[8*gh +: 8]));
        end
        for (gq = 0; gq < 4; gq = gq + 1) begin : g_named
            assign n_G[8*gq +: 8] = c4_yfn ? lr2[8*gq +: 8]   : lr5[8*gq +: 8];
            assign n_H[8*gq +: 8] = c4_yfn ? lr2[8*gq+8 +: 8] : lr5[8*gq+8 +: 8];
            assign n_M[8*gq +: 8] = lr3[8*gq +: 8];
            assign n_b[8*gq +: 8] = clip5(c4_yfn ? lb2[16*gq +: 16] : lb5[16*gq +: 16]);
            assign n_s[8*gq +: 8] = clip5(lb3[16*gq +: 16]);
            assign n_h[8*gq +: 8] = hv[8*gq +: 8];
            assign n_m[8*gq +: 8] = hv[8*gq+8 +: 8];
            assign n_j[8*gq +: 8] = clip10(tap6w(lb0[16*gq +: 16], lb1[16*gq +: 16], lb2[16*gq +: 16],
                                                 lb3[16*gq +: 16], lb4[16*gq +: 16], lb5[16*gq +: 16]));
        end
    endgenerate

    // Chroma's vertical blend, 8-270: ((8 - yF) top + yF bottom + 32) >> 6.
    wire [2:0]  c4_yf = c4[F_YF +: 3];
    wire [31:0] n_c;
    genvar gv;
    generate
        for (gv = 0; gv < 4; gv = gv + 1) begin : g_cv
            wire [10:0] top = cr0[11*gv +: 11];
            wire [10:0] bot = cr1[11*gv +: 11];
            wire [13:0] wt  = mul3(top, 3'd0 - c4_yf);
            wire [13:0] wb  = mul3(bot, c4_yf);
            wire [13:0] cv  = (c4_yf == 3'd0) ? {bot, 3'b000} : wt + wb;
            wire [13:0] cvr = (cv + 14'd32) >> 6;
            assign n_c[8*gv +: 8] = cvr[7:0];
        end
    endgenerate

    reg  [31:0] q_G, q_H, q_M, q_b, q_s, q_h, q_m, q_j, q_c;
    always @(posedge clk) begin
        if (!rst_n) begin
            c4 <= {CW{1'b0}}; c5 <= {CW{1'b0}}; c6 <= {CW{1'b0}};
        end else begin
            c4 <= c3;
            c5 <= c4;
            c6 <= c5;
        end
        q_G <= n_G; q_H <= n_H; q_M <= n_M; q_b <= n_b; q_s <= n_s;
        q_h <= n_h; q_m <= n_m; q_j <= n_j; q_c <= n_c;
    end

    // Stage 5: Table 8-12. Every position is (A + B + 1) >> 1 of two of
    // the named samples, a pure one being the same sample twice.
    wire [1:0]  c5_xf = c5[F_XF +: 2];
    wire [1:0]  c5_yf = c5[F_YF +: 2];
    wire [31:0] n_out;
    genvar gt;
    generate
        for (gt = 0; gt < 4; gt = gt + 1) begin : g_out
            wire [7:0] G = q_G[8*gt +: 8], H = q_H[8*gt +: 8], M = q_M[8*gt +: 8];
            wire [7:0] b = q_b[8*gt +: 8], s = q_s[8*gt +: 8], h = q_h[8*gt +: 8];
            wire [7:0] m = q_m[8*gt +: 8], j = q_j[8*gt +: 8];
            reg  [7:0] pa, pb;
            always @* begin
                case ({c5_xf, c5_yf})
                    4'h0: begin pa = G; pb = G; end  // G
                    4'h1: begin pa = G; pb = h; end  // d
                    4'h2: begin pa = h; pb = h; end  // h
                    4'h3: begin pa = M; pb = h; end  // n
                    4'h4: begin pa = G; pb = b; end  // a
                    4'h5: begin pa = b; pb = h; end  // e
                    4'h6: begin pa = h; pb = j; end  // i
                    4'h7: begin pa = h; pb = s; end  // p
                    4'h8: begin pa = b; pb = b; end  // b
                    4'h9: begin pa = b; pb = j; end  // f
                    4'hA: begin pa = j; pb = j; end  // j
                    4'hB: begin pa = j; pb = s; end  // q
                    4'hC: begin pa = H; pb = b; end  // c
                    4'hD: begin pa = b; pb = m; end  // g
                    4'hE: begin pa = j; pb = m; end  // k
                    default: begin pa = m; pb = s; end  // r
                endcase
            end
            wire [8:0] avg = {1'b0, pa} + {1'b0, pb} + 9'd1;
            assign n_out[8*gt +: 8] = avg[8:1];
        end
    endgenerate

    reg  [31:0] o_px;
    always @(posedge clk) begin
        o_px <= c5[F_LUMA] ? n_out : q_c;
    end

    // ===============================================================
    // The prediction buffer: two macroblocks of 96 words, a byte lane each
    // ===============================================================
    reg  [7:0]  pb0 [0:255];
    reg  [7:0]  pb1 [0:255];
    reg  [7:0]  pb2 [0:255];
    reg  [7:0]  pb3 [0:255];
    wire [7:0]  w_a = {c6[F_BANK], c6[F_WORD +: 7]};
    wire [3:0]  w_l = c6[F_LANES +: 4];
    wire        w_go = c6[F_V] && c6[F_EMIT];
    // Two outputs at lanes 2 and 3 when the first lane is 2.
    wire [31:0] w_d = c6[F_SHIFT + 1] ? {o_px[15:0], o_px[15:0]} : o_px;
    always @(posedge clk) begin
        if (w_go && w_l[0]) pb0[w_a] <= w_d[7:0];
        if (w_go && w_l[1]) pb1[w_a] <= w_d[15:8];
        if (w_go && w_l[2]) pb2[w_a] <= w_d[23:16];
        if (w_go && w_l[3]) pb3[w_a] <= w_d[31:24];
    end

    // ===============================================================
    // Stream
    // ===============================================================
    reg  [1:0]  pb_ready;
    reg         s_bank;
    reg  [6:0]  s_a;
    wire [7:0]  s_ra = {s_bank, s_a};
    reg         s_done;
    assign      pb_done      = s_done;
    assign      pb_done_bank = !s_bank;
    always @(posedge clk) begin
        if (!rst_n) begin
            pb_ready   <= 2'b00;
            s_bank     <= 1'b0;
            s_a        <= 7'd0;
            s_done     <= 1'b0;
            pred_valid <= 1'b0;
            pred_data  <= 32'd0;
        end else begin
            s_done <= 1'b0;
            if (c6[F_V] && c6[F_LAST]) pb_ready[c6[F_BANK]] <= 1'b1;
            if (pred_valid && pred_ready) pred_valid <= 1'b0;
            if (pb_ready[s_bank] && (!pred_valid || pred_ready)) begin
                pred_valid <= 1'b1;
                pred_data  <= {pb3[s_ra], pb2[s_ra], pb1[s_ra], pb0[s_ra]};
                if (s_a == 7'd95) begin
                    s_a      <= 7'd0;
                    pb_ready[s_bank] <= 1'b0;
                    s_bank   <= !s_bank;
                    s_done   <= 1'b1;
                end else begin
                    s_a <= s_a + 7'd1;
                end
            end
        end
    end
endmodule
