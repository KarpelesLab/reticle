// h264_intra — prediction plus residual: the picture construction of
// ITU-T H.264 §8.3 and §8.5.14 for one macroblock at a time, before the
// deblocking filter.
//
// What it does
//   On `go`, takes the macroblock's residuals from `h264_residual` (a
//   row of four a cycle), forms its prediction — one of the nine
//   Intra_4x4 modes per block (8.3.1.2), the four Intra_16x16 modes
//   (8.3.3), the four chroma modes (8.3.4), the inter prediction
//   `h264_inter` streams in, or nothing for I_PCM — and writes
//   Clip1(pred + residual) (8-299, 8-302, 8-308) as words of four
//   samples, column 0 lowest, to `ow_*`:
//     luma   word  y * 4 + x / 4       y = 0..15
//     Cb     word 64 + y * 2 + x / 4   y = 0..7
//     Cr     word 80 + y * 2 + x / 4
//   which `h264_recon` points at one of its four macroblock slots for
//   `h264_deblock`.
//
// Neighbours, and the line buffer they come from
//   Intra prediction reads the constructed samples **before** deblocking
//   (8.3.1.2: "constructed luma samples prior to the deblocking filter
//   process"), so this module keeps its own copy of them rather than
//   reading the picture: the bottom row of every macroblock of the row
//   above, one luma word per four samples (1280 samples: 320 words) and
//   one per four chroma samples of each component (2 x 160), in a
//   640-word memory `lb`; and the right-hand column of the macroblock to
//   the left in `rowr*`. The sample above and to the left (D, p[-1,-1])
//   of a macroblock is the last sample of the word the macroblock to its
//   left read when it started, which is saved then, because by the time
//   it is wanted that macroblock has written its own bottom row over it.
//
//   Which neighbours exist is decided upstream: `avail_a` .. `avail_d`
//   are mbAddrA..D "available for Intra prediction" — inside the
//   picture, in the same slice (6.4.8, no ASO), and not inter when
//   constrained_intra_pred_flag is set (8.3.1.2 step 3). `avail_d` is
//   not read: no mode that may be used without D (8.3.1.2.5's "shall be
//   used only when") reads it, and DC never does.
//
// Intra_4x4, a block at a time
//   The blocks are done in luma4x4BlkIdx order and each sees the ones
//   before it, so the neighbour registers are updated in place: `colb`
//   holds, per column, the bottom sample of the last block constructed
//   in it (the row above the current block), `rowr` per row the right
//   sample of the last block in it, and `cg` the bottom-right sample of
//   each block for the corner of the block diagonally below. A block
//   costs five cycles: one to gather its thirteen neighbours and form
//   every two- and three-tap average of them, four to send its rows.
//
//   All nine modes then become a choice. Lay the neighbours in a line
//   E = L3 L3 L2 L1 L0 C T0 .. T7 T7 (the duplicated ends are 8-52's
//   and 8-69's "3 *"), and let F2[k] = (E[k] + E[k+1] + 1) >> 1 and
//   F3[k] = (E[k-1] + 2 E[k] + E[k+1] + 2) >> 2. Every equation from
//   8-46 to 8-70 is one of E, F2 or F3 at an index that is a function
//   of the mode and (x, y) — `i4_sel` — or the DC.
//
// Everything else, in raster order
//   Intra_16x16, chroma, inter and I_PCM have no dependency inside the
//   macroblock, so they run a word a cycle down the rows: the residual
//   of word (x / 4, y) is fetched from its block on one cycle and added
//   on the next. Plane prediction (8-122, 8-144) is set up once a
//   macroblock — a, b, c and the constant part — and is a multiply and
//   add a sample after that.
//
// Cost: ten cycles to read the neighbours, two to set up, 80 for
// Intra_4x4 luma or 64 for any other, 32 for chroma, two to drain.
module h264_intra #(
    parameter integer MAX_W_MBS = 80
) (
    input  wire        clk,
    input  wire        rst_n,

    input  wire        go,
    output wire        busy,
    output reg         done,

    // The macroblock, held from `go` until `done`.
    input  wire [6:0]  mb_x,
    input  wire [2:0]  kind,              // 0 I4x4, 1 I16x16, 2 inter, 3 I_PCM
    input  wire [1:0]  i16_mode,          // Intra16x16PredMode
    input  wire [1:0]  c_mode,            // intra_chroma_pred_mode
    input  wire [63:0] i4_modes,          // Intra4x4PredMode[b] at [4b +: 4]
    input  wire        avail_a,
    input  wire        avail_b,
    input  wire        avail_c,
    input  wire        avail_d,
    input  wire        half,              // which half of the residual buffer

    // The residual buffer.
    output wire [7:0]  rr_addr,
    input  wire [39:0] rr_data,
    input  wire [23:0] zf,

    // Inter prediction, four samples a word (contract format).
    input  wire        pred_valid,
    output wire        pred_ready,
    input  wire [31:0] pred_data,

    // Constructed samples.
    output reg         ow_en,
    output reg  [6:0]  ow_addr,
    output reg  [31:0] ow_data
);
    localparam [2:0] K_I4  = 3'd0;
    localparam [2:0] K_I16 = 3'd1;
    localparam [2:0] K_INT = 3'd2;
    localparam [2:0] K_PCM = 3'd3;

    // -----------------------------------------------------------------
    // The line buffer of bottom rows
    // -----------------------------------------------------------------
    localparam integer LB_CB = MAX_W_MBS * 4;
    localparam integer LB_CR = MAX_W_MBS * 6;
    localparam integer LB_N  = MAX_W_MBS * 8;
    localparam [9:0] LB_CB10 = LB_CB;
    localparam [9:0] LB_CR10 = LB_CR;

    reg [31:0] lb [0:LB_N-1];
    reg [31:0] lb_q;
    reg        lb_we;
    reg [9:0]  lb_wa;
    reg [31:0] lb_wd;
    reg [9:0]  lb_ra;

    integer n;
    initial begin
        for (n = 0; n < LB_N; n = n + 1) lb[n] = 32'd0;
    end

    always @(posedge clk) begin
        if (lb_we) lb[lb_wa] <= lb_wd;
        lb_q <= lb[lb_ra];
    end

    // -----------------------------------------------------------------
    // Neighbour registers
    // -----------------------------------------------------------------
    reg [127:0] colb;          // luma: the row above / bottom of last block, per column
    reg [31:0]  ar;            // luma p[16..19, -1], from the macroblock above-right
    reg [127:0] rowr;          // luma: the column left / right of last block, per row
    reg [127:0] rowr_n;        // this macroblock's right column, for the next one
    reg [127:0] cg;            // block corners: byte 4 by + bx is p[4 bx - 1, 4 by - 1]
    reg [7:0]   dl, dcb, dcr;  // p[-1, -1] of each component
    reg [7:0]   nd_l, nd_cb, nd_cr;   // ... of the next macroblock
    reg [63:0]  colb_cb, colb_cr;
    reg [63:0]  rowr_cb, rowr_cr, rowr_cb_n, rowr_cr_n;

    function [7:0] byte16;
        input [127:0] v;
        input [3:0]   i;
        begin
            byte16 = v[8 * i +: 8];
        end
    endfunction

    function [7:0] byte8;
        input [63:0] v;
        input [2:0]  i;
        begin
            byte8 = v[8 * i +: 8];
        end
    endfunction

    // -----------------------------------------------------------------
    // Control
    // -----------------------------------------------------------------
    localparam [2:0] ST_IDLE  = 3'd0;
    localparam [2:0] ST_LOAD  = 3'd1;
    localparam [2:0] ST_SETUP = 3'd2;
    localparam [2:0] ST_LUMA  = 3'd3;
    localparam [2:0] ST_CHR   = 3'd4;
    localparam [2:0] ST_FLUSH = 3'd5;

    reg [2:0] state;
    reg [3:0] ld;              // load step
    reg       su;              // setup step
    reg [1:0] fl;              // flush step

    // S0, the fetch: which word comes next.
    reg [3:0] s0_y;
    reg [1:0] s0_w;
    reg [1:0] s0_p;            // 0 luma, 1 Cb, 2 Cr
    reg [3:0] s0_k;            // Intra_4x4 block
    reg [2:0] s0_r;            // Intra_4x4 row; 4 is the cycle after row 3

    // S1, the add.
    reg        s1_v;
    reg [3:0]  s1_y;
    reg [1:0]  s1_w;
    reg [1:0]  s1_p;
    reg        s1_zero;
    reg [31:0] s1_pred;
    reg        s1_i4;
    reg [1:0]  s1_r;

    assign busy = (state != ST_IDLE);

    wire is_i4  = (kind == K_I4);
    wire is_int = (kind == K_INT);

    // Raster fetch in progress, and whether it may move this cycle.
    wire raster_luma = (state == ST_LUMA) && !is_i4;
    wire raster      = raster_luma || (state == ST_CHR);
    wire s0_go       = raster && (!is_int || pred_valid);
    assign pred_ready = raster && is_int;

    // Intra_4x4 fetch: rows 0..3 of block s0_k.
    wire i4_fetch = (state == ST_LUMA) && is_i4 && (s0_r != 3'd4);

    // The block and row of the word being fetched.
    wire [3:0] rast_blk  = {s0_y[3], s0_w[1], s0_y[2], s0_w[0]};
    wire [4:0] fetch_blk = i4_fetch ? {1'b0, s0_k} :
                           (s0_p == 2'd0) ? {1'b0, rast_blk} :
                           (s0_p == 2'd1) ? {3'b100, s0_y[2], s0_w[0]} :
                                            {3'b101, s0_y[2], s0_w[0]};
    wire [1:0] fetch_row = i4_fetch ? s0_r[1:0] : s0_y[1:0];
    assign rr_addr = {half, fetch_blk, fetch_row};

    // -----------------------------------------------------------------
    // Intra_4x4: the block's neighbours and their averages
    // -----------------------------------------------------------------
    wire [1:0] bx = {s0_k[2], s0_k[0]};
    wire [1:0] by = {s0_k[3], s0_k[1]};

    // luma4x4BlkIdx of the block at (x, y) in 4-sample units (6.4.3).
    function [3:0] blk_at;
        input [1:0] x;
        input [1:0] y;
        begin
            blk_at = {y[1], x[1], y[0], x[0]};
        end
    endfunction

    // Is p[4..7, -1] available? Above the macroblock it is B's or C's;
    // inside, the block up and to the right must come earlier in
    // luma4x4BlkIdx order — which is what leaves blocks 3, 7, 11, 13
    // and 15 without: 3 and 11 are 8.3.1.2's named exceptions, the rest
    // fall outside the macroblock to the right (6.4.12).
    wire [1:0] bx1 = bx + 2'd1;
    wire [1:0] bym = by - 2'd1;
    wire tr_avail = (by == 2'd0) ? ((bx == 2'd3) ? avail_c : avail_b) :
                    ((bx != 2'd3) && (blk_at(bx1, bym) < s0_k));
    wire top_avail  = (by != 2'd0) || avail_b;
    wire left_avail = (bx != 2'd0) || avail_a;
    wire from_ar    = (by == 2'd0) && (bx == 2'd3);

    wire [7:0] t0 = byte16(colb, {bx, 2'd0});
    wire [7:0] t1 = byte16(colb, {bx, 2'd1});
    wire [7:0] t2 = byte16(colb, {bx, 2'd2});
    wire [7:0] t3 = byte16(colb, {bx, 2'd3});
    wire [7:0] tr0 = from_ar ? ar[7:0]   : byte16(colb, {bx1, 2'd0});
    wire [7:0] tr1 = from_ar ? ar[15:8]  : byte16(colb, {bx1, 2'd1});
    wire [7:0] tr2 = from_ar ? ar[23:16] : byte16(colb, {bx1, 2'd2});
    wire [7:0] tr3 = from_ar ? ar[31:24] : byte16(colb, {bx1, 2'd3});
    // 8.3.1.2: p[3, -1] stands in for p[4..7, -1] when they are missing.
    wire [7:0] t4 = tr_avail ? tr0 : t3;
    wire [7:0] t5 = tr_avail ? tr1 : t3;
    wire [7:0] t6 = tr_avail ? tr2 : t3;
    wire [7:0] t7 = tr_avail ? tr3 : t3;
    wire [7:0] l0 = byte16(rowr, {by, 2'd0});
    wire [7:0] l1 = byte16(rowr, {by, 2'd1});
    wire [7:0] l2 = byte16(rowr, {by, 2'd2});
    wire [7:0] l3 = byte16(rowr, {by, 2'd3});
    wire [7:0] cc = byte16(cg, {by, bx});

    // E[0..14], E[k] at [8k +: 8].
    wire [119:0] e = {t7, t7, t6, t5, t4, t3, t2, t1, t0, cc, l0, l1, l2, l3, l3};

    function [7:0] e_at;
        input [119:0] v;
        input [3:0]   k;
        begin
            e_at = v[8 * k +: 8];
        end
    endfunction

    // F2[k] = (E[k] + E[k+1] + 1) >> 1, k = 0..13; F3[k], k = 1..13
    // (F3[0] is never chosen and is left zero).
    reg [111:0] f2_now;
    reg [111:0] f3_now;
    reg [9:0]   tap;
    integer ia,ib,ic,id,ie,ig;
    always @* begin
        f2_now = 112'd0;
        f3_now = 112'd0;
        for (ia = 0; ia < 14; ia = ia + 1) begin
            tap = {2'd0, e[8 * ia +: 8]} + {2'd0, e[8 * (ia + 1) +: 8]} + 10'd1;
            f2_now[8 * ia +: 8] = tap[8:1];
        end
        for (ia = 1; ia < 14; ia = ia + 1) begin
            tap = {2'd0, e[8 * (ia - 1) +: 8]} + {1'd0, e[8 * ia +: 8], 1'b0} +
                  {2'd0, e[8 * (ia + 1) +: 8]} + 10'd2;
            f3_now[8 * ia +: 8] = tap[9:2];
        end
    end

    wire [9:0]  sum_t4 = {2'd0, t0} + {2'd0, t1} + {2'd0, t2} + {2'd0, t3};
    wire [9:0]  sum_l4 = {2'd0, l0} + {2'd0, l1} + {2'd0, l2} + {2'd0, l3};
    wire [10:0] sum_tl = {1'b0, sum_t4} + {1'b0, sum_l4} + 11'd4;
    wire [9:0]  sum_t2 = sum_t4 + 10'd2;
    wire [9:0]  sum_l2 = sum_l4 + 10'd2;
    // 8-48 .. 8-51.
    wire [7:0] dc4_now = (top_avail && left_avail) ? sum_tl[10:3] :
                         left_avail ? sum_l2[9:2] :
                         top_avail  ? sum_t2[9:2] : 8'd128;

    reg [119:0] e_q;
    reg [111:0] f2_q;
    reg [111:0] f3_q;
    reg [7:0]   dc4_q;
    reg [3:0]   mode_q;
    reg [1:0]   bx_q, by_q;

    // Which of E, F2, F3 or the DC a sample takes: {type[1:0], index[3:0]},
    // type 0 E, 1 F2, 2 F3, 3 DC.
    function [5:0] i4_sel;
        input [3:0] mode;
        input [1:0] x;
        input [1:0] y;
        reg [3:0] xi, yi, hy, hx;
        reg signed [4:0] z;
        begin
            xi = {2'd0, x};
            yi = {2'd0, y};
            hy = {3'd0, y[1]};
            hx = {3'd0, x[1]};
            case (mode)
                // 8-46, Vertical.
                4'd0: i4_sel = {2'd0, 4'd6 + xi};
                // 8-47, Horizontal.
                4'd1: i4_sel = {2'd0, 4'd4 - yi};
                // 8-48 .. 8-51, DC.
                4'd2: i4_sel = {2'd3, 4'd0};
                // 8-52, 8-53, Diagonal_Down_Left.
                4'd3: i4_sel = {2'd2, 4'd7 + xi + yi};
                // 8-54 .. 8-56, Diagonal_Down_Right.
                4'd4: i4_sel = {2'd2, 4'd5 + xi - yi};
                // 8-57 .. 8-60, Vertical_Right: zVR = 2x - y.
                4'd5: begin
                    z = $signed({1'b0, xi}) * 5'sd2 - $signed({1'b0, yi});
                    if (z >= 5'sd0 && !z[0])
                        i4_sel = {2'd1, 4'd5 + xi - hy};
                    else if (z >= -5'sd1)
                        i4_sel = {2'd2, 4'd5 + xi - hy};
                    else
                        i4_sel = {2'd2, 4'd6 - yi};
                end
                // 8-61 .. 8-64, Horizontal_Down: zHD = 2y - x.
                4'd6: begin
                    z = $signed({1'b0, yi}) * 5'sd2 - $signed({1'b0, xi});
                    if (z >= 5'sd0 && !z[0])
                        i4_sel = {2'd1, 4'd4 - yi + hx};
                    else if (z >= -5'sd1)
                        i4_sel = {2'd2, 4'd5 - yi + hx};
                    else
                        i4_sel = {2'd2, 4'd4 + xi};
                end
                // 8-65, 8-66, Vertical_Left.
                4'd7: begin
                    if (!y[0]) i4_sel = {2'd1, 4'd6 + xi + hy};
                    else       i4_sel = {2'd2, 4'd7 + xi + hy};
                end
                // 8-67 .. 8-70, Horizontal_Up: zHU = x + 2y.
                4'd8: begin
                    z = $signed({1'b0, xi}) + $signed({1'b0, yi}) * 5'sd2;
                    if (z > 5'sd5)
                        i4_sel = {2'd0, 4'd1};
                    else if (!z[0])
                        i4_sel = {2'd1, 4'd3 - yi - hx};
                    else
                        i4_sel = {2'd2, 4'd3 - yi - hx};
                end
                default: i4_sel = {2'd3, 4'd0};
            endcase
        end
    endfunction

    function [7:0] i4_pick;
        input [5:0]   sel;
        input [119:0] ev;
        input [111:0] f2v;
        input [111:0] f3v;
        input [7:0]   dcv;
        begin
            case (sel[5:4])
                2'd0:    i4_pick = ev[8 * sel[3:0] +: 8];
                2'd1:    i4_pick = f2v[8 * sel[3:0] +: 8];
                2'd2:    i4_pick = f3v[8 * sel[3:0] +: 8];
                default: i4_pick = dcv;
            endcase
        end
    endfunction

    // -----------------------------------------------------------------
    // Intra_16x16 and chroma: set up once a macroblock
    // -----------------------------------------------------------------
    // Sums of the sixteen above and sixteen left.
    reg [11:0] s16t, s16l;
    always @* begin
        s16t = 12'd0;
        s16l = 12'd0;
        for (ib = 0; ib < 16; ib = ib + 1) begin
            s16t = s16t + {4'd0, colb[8 * ib +: 8]};
            s16l = s16l + {4'd0, rowr[8 * ib +: 8]};
        end
    end

    // A neighbour as a signed number, p[-1] being the corner.
    function signed [9:0] nb16;
        input [127:0] v;
        input [7:0]   d;
        input integer k;
        begin
            if (k < 0) nb16 = $signed({2'b00, d});
            else       nb16 = $signed({2'b00, v[8 * k +: 8]});
        end
    endfunction

    function signed [9:0] nb8;
        input [63:0] v;
        input [7:0]  d;
        input integer k;
        begin
            if (k < 0) nb8 = $signed({2'b00, d});
            else       nb8 = $signed({2'b00, v[8 * k +: 8]});
        end
    endfunction

    // 8-126 / 8-127.
    reg signed [19:0] h16, v16;
    always @* begin
        h16 = 20'sd0;
        v16 = 20'sd0;
        for (ic = 0; ic < 8; ic = ic + 1) begin
            h16 = h16 + (ic + 1) * (nb16(colb, dl, 8 + ic) - nb16(colb, dl, 6 - ic));
            v16 = v16 + (ic + 1) * (nb16(rowr, dl, 8 + ic) - nb16(rowr, dl, 6 - ic));
        end
    end

    // 8-148 / 8-149, 4:2:0: xCF = yCF = 0.
    reg signed [19:0] hcb, vcb, hcr, vcr;
    always @* begin
        hcb = 20'sd0; vcb = 20'sd0; hcr = 20'sd0; vcr = 20'sd0;
        for (id = 0; id < 4; id = id + 1) begin
            hcb = hcb + (id + 1) * (nb8(colb_cb, dcb, 4 + id) - nb8(colb_cb, dcb, 2 - id));
            vcb = vcb + (id + 1) * (nb8(rowr_cb, dcb, 4 + id) - nb8(rowr_cb, dcb, 2 - id));
            hcr = hcr + (id + 1) * (nb8(colb_cr, dcr, 4 + id) - nb8(colb_cr, dcr, 2 - id));
            vcr = vcr + (id + 1) * (nb8(rowr_cr, dcr, 4 + id) - nb8(rowr_cr, dcr, 2 - id));
        end
    end

    // Chroma DC, 8-132 .. 8-141, for one component's block k.
    function [7:0] cdc;
        input [63:0] top;
        input [63:0] left;
        input [1:0]  k;
        input        a;
        input        b;
        reg [9:0] st, sl;
        reg [10:0] both;
        reg [9:0] t2, l2;
        begin
            st = {2'd0, top[8 * {k[0], 2'd0} +: 8]} + {2'd0, top[8 * {k[0], 2'd1} +: 8]} +
                 {2'd0, top[8 * {k[0], 2'd2} +: 8]} + {2'd0, top[8 * {k[0], 2'd3} +: 8]};
            sl = {2'd0, left[8 * {k[1], 2'd0} +: 8]} + {2'd0, left[8 * {k[1], 2'd1} +: 8]} +
                 {2'd0, left[8 * {k[1], 2'd2} +: 8]} + {2'd0, left[8 * {k[1], 2'd3} +: 8]};
            both = {1'b0, st} + {1'b0, sl} + 11'd4;
            t2 = st + 10'd2;
            l2 = sl + 10'd2;
            if (k == 2'd1) begin
                // xO > 0, yO = 0: the samples above first.
                if (b)      cdc = t2[9:2];
                else if (a) cdc = l2[9:2];
                else        cdc = 8'd128;
            end else if (k == 2'd2) begin
                // xO = 0, yO > 0: the samples to the left first.
                if (a)      cdc = l2[9:2];
                else if (b) cdc = t2[9:2];
                else        cdc = 8'd128;
            end else begin
                if (a && b) cdc = both[10:3];
                else if (a) cdc = l2[9:2];
                else if (b) cdc = t2[9:2];
                else        cdc = 8'd128;
            end
        end
    endfunction

    reg [7:0]  dc16;
    reg [31:0] dcc_cb, dcc_cr;            // block k at [8k +: 8]
    reg signed [19:0] h16_q, v16_q, hcb_q, vcb_q, hcr_q, vcr_q;
    reg signed [19:0] pl_b, pl_c, pl_base;   // luma plane
    reg signed [19:0] cb_b, cb_c, cb_base;
    reg signed [19:0] cr_b, cr_c, cr_base;

    wire signed [19:0] pl_b_now = (h16_q * 20'sd5 + 20'sd32) >>> 6;
    wire signed [19:0] pl_c_now = (v16_q * 20'sd5 + 20'sd32) >>> 6;
    wire signed [19:0] cb_b_now = (hcb_q * 20'sd34 + 20'sd32) >>> 6;
    wire signed [19:0] cb_c_now = (vcb_q * 20'sd34 + 20'sd32) >>> 6;
    wire signed [19:0] cr_b_now = (hcr_q * 20'sd34 + 20'sd32) >>> 6;
    wire signed [19:0] cr_c_now = (vcr_q * 20'sd34 + 20'sd32) >>> 6;
    wire signed [19:0] pl_base_now =
        20'sd16 * ($signed({12'd0, rowr[127:120]}) + $signed({12'd0, colb[127:120]}))
        - 20'sd7 * pl_b_now - 20'sd7 * pl_c_now + 20'sd16;
    wire signed [19:0] cb_base_now =
        20'sd16 * ($signed({12'd0, rowr_cb[63:56]}) + $signed({12'd0, colb_cb[63:56]}))
        - 20'sd3 * cb_b_now - 20'sd3 * cb_c_now + 20'sd16;
    wire signed [19:0] cr_base_now =
        20'sd16 * ($signed({12'd0, rowr_cr[63:56]}) + $signed({12'd0, colb_cr[63:56]}))
        - 20'sd3 * cr_b_now - 20'sd3 * cr_c_now + 20'sd16;

    wire [12:0] dc16_both = {1'b0, s16t} + {1'b0, s16l} + 13'd16;
    wire [11:0] dc16_t    = s16t + 12'd8;
    wire [11:0] dc16_l    = s16l + 12'd8;

    // -----------------------------------------------------------------
    // S1: the prediction of the word in flight
    // -----------------------------------------------------------------
    function [7:0] clip1;
        input signed [20:0] v;
        begin
            if (v < 21'sd0)        clip1 = 8'd0;
            else if (v > 21'sd255) clip1 = 8'd255;
            else                   clip1 = v[7:0];
        end
    endfunction

    reg [31:0] pred_w;
    reg [3:0]  lx;
    reg signed [20:0] pv;
    always @* begin
        pred_w = 32'd0;
        for (ie = 0; ie < 4; ie = ie + 1) begin
            lx = {s1_w, 2'd0} + ie;
            if (s1_i4) begin
                pred_w[8 * ie +: 8] = i4_pick(i4_sel(mode_q, ie, s1_r), e_q, f2_q, f3_q, dc4_q);
            end else if (kind == K_INT) begin
                pred_w[8 * ie +: 8] = s1_pred[8 * ie +: 8];
            end else if (kind == K_PCM) begin
                pred_w[8 * ie +: 8] = 8'd0;
            end else if (s1_p == 2'd0) begin
                case (i16_mode)
                    2'd0: pred_w[8 * ie +: 8] = colb[8 * lx +: 8];
                    2'd1: pred_w[8 * ie +: 8] = rowr[8 * s1_y +: 8];
                    2'd2: pred_w[8 * ie +: 8] = dc16;
                    default: begin
                        pv = (pl_base + pl_b * $signed({1'b0, lx}) +
                              pl_c * $signed({1'b0, s1_y})) >>> 5;
                        pred_w[8 * ie +: 8] = clip1(pv);
                    end
                endcase
            end else begin
                case (c_mode)
                    2'd0: pred_w[8 * ie +: 8] = (s1_p == 2'd1) ?
                              dcc_cb[8 * {s1_y[2], s1_w[0]} +: 8] :
                              dcc_cr[8 * {s1_y[2], s1_w[0]} +: 8];
                    2'd1: pred_w[8 * ie +: 8] = (s1_p == 2'd1) ?
                              rowr_cb[8 * s1_y[2:0] +: 8] : rowr_cr[8 * s1_y[2:0] +: 8];
                    2'd2: pred_w[8 * ie +: 8] = (s1_p == 2'd1) ?
                              colb_cb[8 * lx[2:0] +: 8] : colb_cr[8 * lx[2:0] +: 8];
                    default: begin
                        if (s1_p == 2'd1)
                            pv = (cb_base + cb_b * $signed({1'b0, lx}) +
                                  cb_c * $signed({1'b0, s1_y})) >>> 5;
                        else
                            pv = (cr_base + cr_b * $signed({1'b0, lx}) +
                                  cr_c * $signed({1'b0, s1_y})) >>> 5;
                        pred_w[8 * ie +: 8] = clip1(pv);
                    end
                endcase
            end
        end
    end

    // Prediction plus residual, clipped.
    reg [31:0] rec_w;
    reg signed [20:0] sum;
    reg signed [9:0]  res;
    always @* begin
        for (ig = 0; ig < 4; ig = ig + 1) begin
            res = s1_zero ? 10'sd0 : $signed(rr_data[10 * ig +: 10]);
            sum = $signed({13'd0, pred_w[8 * ig +: 8]}) + {{11{res[9]}}, res};
            rec_w[8 * ig +: 8] = clip1(sum);
        end
    end

    // Where the word in S1 belongs.
    wire [3:0] s1_ya = s1_i4 ? {by_q, s1_r} : s1_y;     // row in its plane
    wire [1:0] s1_wa = s1_i4 ? bx_q : s1_w;             // word in its row
    wire [6:0] s1_addr = (s1_p == 2'd0) ? {1'b0, s1_ya, s1_wa} :
                         (s1_p == 2'd1) ? (7'd64 + {2'd0, s1_ya[2:0], s1_wa[0]}) :
                                          (7'd80 + {2'd0, s1_ya[2:0], s1_wa[0]});

    // -----------------------------------------------------------------
    // The sequence
    // -----------------------------------------------------------------
    wire [9:0] mbx10 = {3'd0, mb_x};

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state <= ST_IDLE;
            ld <= 4'd0; su <= 1'b0; fl <= 2'd0;
            done <= 1'b0;
            s0_y <= 4'd0; s0_w <= 2'd0; s0_p <= 2'd0; s0_k <= 4'd0; s0_r <= 3'd0;
            s1_v <= 1'b0; s1_y <= 4'd0; s1_w <= 2'd0; s1_p <= 2'd0; s1_zero <= 1'b0;
            s1_pred <= 32'd0; s1_i4 <= 1'b0; s1_r <= 2'd0;
            colb <= 128'd0; ar <= 32'd0; rowr <= 128'd0; rowr_n <= 128'd0; cg <= 128'd0;
            dl <= 8'd0; dcb <= 8'd0; dcr <= 8'd0; nd_l <= 8'd0; nd_cb <= 8'd0; nd_cr <= 8'd0;
            colb_cb <= 64'd0; colb_cr <= 64'd0;
            rowr_cb <= 64'd0; rowr_cr <= 64'd0; rowr_cb_n <= 64'd0; rowr_cr_n <= 64'd0;
            e_q <= 120'd0; f2_q <= 112'd0; f3_q <= 112'd0; dc4_q <= 8'd0; mode_q <= 4'd0;
            bx_q <= 2'd0; by_q <= 2'd0;
            dc16 <= 8'd0; dcc_cb <= 32'd0; dcc_cr <= 32'd0;
            h16_q <= 20'sd0; v16_q <= 20'sd0; hcb_q <= 20'sd0; vcb_q <= 20'sd0;
            hcr_q <= 20'sd0; vcr_q <= 20'sd0;
            pl_b <= 20'sd0; pl_c <= 20'sd0; pl_base <= 20'sd0;
            cb_b <= 20'sd0; cb_c <= 20'sd0; cb_base <= 20'sd0;
            cr_b <= 20'sd0; cr_c <= 20'sd0; cr_base <= 20'sd0;
            lb_we <= 1'b0; lb_wa <= 10'd0; lb_wd <= 32'd0; lb_ra <= 10'd0;
            ow_en <= 1'b0; ow_addr <= 7'd0; ow_data <= 32'd0;
        end else begin
            done  <= 1'b0;
            lb_we <= 1'b0;
            ow_en <= 1'b0;

            // ---- S1: add, write, remember ----
            if (s1_v) begin
                ow_en   <= 1'b1;
                ow_addr <= s1_addr;
                ow_data <= rec_w;
                if (s1_p == 2'd0) begin
                    if (s1_ya == 4'd15) begin
                        lb_we <= 1'b1;
                        lb_wa <= {mbx10[7:0], s1_wa};
                        lb_wd <= rec_w;
                    end
                    if (s1_wa == 2'd3)
                        rowr_n[8 * s1_ya +: 8] <= rec_w[31:24];
                    if (s1_i4) begin
                        rowr[8 * s1_ya +: 8] <= rec_w[31:24];
                        if (s1_r == 2'd3) begin
                            colb[32 * bx_q +: 32] <= rec_w;
                            if (bx_q != 2'd3 && by_q != 2'd3)
                                cg[8 * {by_q + 2'd1, bx_q + 2'd1} +: 8] <= rec_w[31:24];
                        end
                    end
                end else begin
                    if (s1_ya[2:0] == 3'd7) begin
                        lb_we <= 1'b1;
                        lb_wa <= ((s1_p == 2'd1) ? LB_CB10 : LB_CR10) +
                                 {mbx10[8:0], s1_wa[0]};
                        lb_wd <= rec_w;
                    end
                    if (s1_wa[0]) begin
                        if (s1_p == 2'd1) rowr_cb_n[8 * s1_ya[2:0] +: 8] <= rec_w[31:24];
                        else              rowr_cr_n[8 * s1_ya[2:0] +: 8] <= rec_w[31:24];
                    end
                end
            end

            // ---- S0 -> S1 ----
            s1_v <= 1'b0;
            if (s0_go || i4_fetch) begin
                s1_v    <= 1'b1;
                s1_y    <= s0_y;
                s1_w    <= s0_w;
                s1_p    <= s0_p;
                s1_zero <= zf[fetch_blk];
                s1_pred <= pred_data;
                s1_i4   <= i4_fetch;
                s1_r    <= s0_r[1:0];
            end

            case (state)
                ST_IDLE: if (go) begin
                    state <= ST_LOAD;
                    ld    <= 4'd0;
                    lb_ra <= {mbx10[7:0], 2'd0};
                end

                // Ten reads of the line buffer, one a cycle; each word
                // lands the cycle after its address.
                ST_LOAD: begin
                    ld <= ld + 4'd1;
                    case (ld)
                        4'd0: lb_ra <= {mbx10[7:0], 2'd1};
                        4'd1: begin
                            colb[31:0] <= lb_q;
                            lb_ra <= {mbx10[7:0], 2'd2};
                        end
                        4'd2: begin
                            colb[63:32] <= lb_q;
                            lb_ra <= {mbx10[7:0], 2'd3};
                        end
                        4'd3: begin
                            colb[95:64] <= lb_q;
                            lb_ra <= {mbx10[7:0] + 8'd1, 2'd0};
                        end
                        4'd4: begin
                            colb[127:96] <= lb_q;
                            dl   <= nd_l;
                            nd_l <= lb_q[31:24];
                            lb_ra <= LB_CB10 + {mbx10[8:0], 1'b0};
                        end
                        4'd5: begin
                            ar <= lb_q;
                            lb_ra <= LB_CB10 + {mbx10[8:0], 1'b1};
                        end
                        4'd6: begin
                            colb_cb[31:0] <= lb_q;
                            lb_ra <= LB_CR10 + {mbx10[8:0], 1'b0};
                        end
                        4'd7: begin
                            colb_cb[63:32] <= lb_q;
                            dcb   <= nd_cb;
                            nd_cb <= lb_q[31:24];
                            lb_ra <= LB_CR10 + {mbx10[8:0], 1'b1};
                        end
                        4'd8: begin
                            colb_cr[31:0] <= lb_q;
                        end
                        default: begin
                            colb_cr[63:32] <= lb_q;
                            dcr   <= nd_cr;
                            nd_cr <= lb_q[31:24];
                            state <= ST_SETUP;
                            su    <= 1'b0;
                        end
                    endcase
                end

                ST_SETUP: begin
                    su <= 1'b1;
                    if (!su) begin
                        // The corners the macroblock's edge gives.
                        cg[7:0]     <= dl;
                        cg[15:8]    <= colb[31:24];
                        cg[23:16]   <= colb[63:56];
                        cg[31:24]   <= colb[95:88];
                        cg[39:32]   <= rowr[31:24];
                        cg[71:64]   <= rowr[63:56];
                        cg[103:96]  <= rowr[95:88];
                        h16_q <= h16; v16_q <= v16;
                        hcb_q <= hcb; vcb_q <= vcb; hcr_q <= hcr; vcr_q <= vcr;
                        // 8-118 .. 8-121.
                        if (avail_a && avail_b) dc16 <= dc16_both[12:5];
                        else if (avail_a)       dc16 <= dc16_l[11:4];
                        else if (avail_b)       dc16 <= dc16_t[11:4];
                        else                    dc16 <= 8'd128;
                        dcc_cb <= {cdc(colb_cb, rowr_cb, 2'd3, avail_a, avail_b),
                                   cdc(colb_cb, rowr_cb, 2'd2, avail_a, avail_b),
                                   cdc(colb_cb, rowr_cb, 2'd1, avail_a, avail_b),
                                   cdc(colb_cb, rowr_cb, 2'd0, avail_a, avail_b)};
                        dcc_cr <= {cdc(colb_cr, rowr_cr, 2'd3, avail_a, avail_b),
                                   cdc(colb_cr, rowr_cr, 2'd2, avail_a, avail_b),
                                   cdc(colb_cr, rowr_cr, 2'd1, avail_a, avail_b),
                                   cdc(colb_cr, rowr_cr, 2'd0, avail_a, avail_b)};
                    end else begin
                        // 8-123 .. 8-125 and 8-145 .. 8-147, with the
                        // constant part of 8-122 / 8-144 folded in.
                        pl_b    <= pl_b_now;
                        pl_c    <= pl_c_now;
                        cb_b    <= cb_b_now;
                        cb_c    <= cb_c_now;
                        cr_b    <= cr_b_now;
                        cr_c    <= cr_c_now;
                        pl_base <= pl_base_now;
                        cb_base <= cb_base_now;
                        cr_base <= cr_base_now;
                        state   <= ST_LUMA;
                        s0_y <= 4'd0; s0_w <= 2'd0; s0_p <= 2'd0;
                        s0_k <= 4'd0; s0_r <= 3'd0;
                    end
                end

                ST_LUMA: begin
                    if (is_i4) begin
                        if (s0_r == 3'd0) begin
                            e_q    <= e;
                            f2_q   <= f2_now;
                            f3_q   <= f3_now;
                            dc4_q  <= dc4_now;
                            mode_q <= i4_modes[4 * s0_k +: 4];
                            bx_q   <= bx;
                            by_q   <= by;
                        end
                        if (s0_r == 3'd4) begin
                            s0_r <= 3'd0;
                            s0_k <= s0_k + 4'd1;
                            if (s0_k == 4'd15) begin
                                state <= ST_CHR;
                                s0_p <= 2'd1; s0_y <= 4'd0; s0_w <= 2'd0;
                            end
                        end else begin
                            s0_r <= s0_r + 3'd1;
                        end
                    end else if (s0_go) begin
                        s0_w <= s0_w + 2'd1;
                        if (s0_w == 2'd3) begin
                            s0_y <= s0_y + 4'd1;
                            if (s0_y == 4'd15) begin
                                state <= ST_CHR;
                                s0_p <= 2'd1; s0_y <= 4'd0; s0_w <= 2'd0;
                            end
                        end
                    end
                end

                ST_CHR: if (s0_go) begin
                    s0_w <= {1'b0, ~s0_w[0]};
                    if (s0_w[0]) begin
                        s0_y <= s0_y + 4'd1;
                        if (s0_y == 4'd7) begin
                            s0_y <= 4'd0;
                            if (s0_p == 2'd2) begin
                                state <= ST_FLUSH;
                                fl    <= 2'd0;
                            end else begin
                                s0_p <= 2'd2;
                            end
                        end
                    end
                end

                ST_FLUSH: begin
                    // The last word is in S1 on the first cycle here and
                    // its right sample in rowr*_n after it.
                    fl <= fl + 2'd1;
                    if (fl == 2'd1) begin
                        rowr    <= rowr_n;
                        rowr_cb <= rowr_cb_n;
                        rowr_cr <= rowr_cr_n;
                        state   <= ST_IDLE;
                        done    <= 1'b1;
                    end
                end

                default: state <= ST_IDLE;
            endcase
        end
    end

endmodule
