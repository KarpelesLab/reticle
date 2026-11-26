// h264_residual — coefficient levels in, residual samples out: the
// scaling and inverse transforms of ITU-T H.264 §8.5 for one macroblock
// at a time.
//
// What it does
//   Stage A of `h264_recon` (`h264_tokin`) writes a macroblock's
//   transform coefficient levels into one half of the coefficient
//   memory here, each at its **raster** position inside its 4x4 block
//   (the inverse zig-zag of §8.5.6 is done by the writer), and a 16-bit
//   mask per block saying which positions it wrote. This module then,
//   on `go`, turns that half into a macroblock of residuals in one half
//   of the residual buffer, which stage B2 (`h264_intra`) reads a row of
//   four residuals at a time.
//
//   The coefficient memory is never cleared. A position whose mask bit
//   is clear reads as zero, and a block nobody wrote this macroblock has
//   its `coded` bit clear, so it is skipped altogether: its flag in
//   `zf0`/`zf1` says "all zero" and B2 adds nothing. Most blocks of most
//   inter macroblocks are like that, and skipping them is most of why
//   the average cost per macroblock is far below the worst case.
//
// Block numbering (the coefficient memory's `blk`)
//   0..15  luma 4x4 blocks, luma4x4BlkIdx (§6.4.3's zig-zag of 8x8s)
//   16..19 Cb 4x4 blocks, chroma4x4BlkIdx; 20..23 Cr likewise
//   24     Intra16x16DCLevel, raster positions of the 4x4 array c of 8.5.2
//   25     Cb ChromaDCLevel, positions 0..3 = c00 c01 c10 c11 (8-304)
//   26     Cr ChromaDCLevel, likewise
//   For an I_PCM macroblock the same memory holds the samples: luma
//   sample (x, y) in block luma4x4BlkIdx of (x/4, y/4) at (y%4)*4 + x%4,
//   chroma likewise in blocks 16..23. They are copied through untouched
//   (8.3.5) and B2 adds them to a prediction of zero.
//
// What each macroblock kind gets (`kind`)
//   0  Intra_4x4, 2 inter: every block by 8.5.12 with its own c00.
//   1  Intra_16x16: the luma DC block by 8.5.10 first (the Hadamard on
//      `h264_itrans4`, then 8-321 / 8-322), and each luma block's c00
//      replaced by its dcY (8-300, 8-335: d00 = c00, unscaled again).
//   3  I_PCM: the copy above.
//   Chroma, in every kind but I_PCM: the 2x2 transform and scaling of
//   8.5.11 (8-324, 8-326) on each component's DC, and each chroma
//   block's c00 replaced by its dcC (8-306).
//
// Scaling, 8.5.12.1
//   d = (c * LevelScale4x4(qP % 6, i, j)) << (qP / 6 - 4)     qP >= 24
//   d = (c * LevelScale4x4 + 2^(3 - qP/6)) >> (4 - qP / 6)    qP <  24
//   with LevelScale4x4 = weightScale4x4(i, j) * normAdjust4x4 (8-313).
//   **The weights are the flat 16 of Flat_4x4_16**, which is what every
//   stream without scaling matrices (all of Baseline and Main) uses.
//   `weight` below is the one place a scaling list would enter: High
//   profile's ScalingList4x4[iYCbCr + 3 * mbIsInterFlag] read through
//   the inverse zig-zag (8.5.9), and the 8x8 transform's own LevelScale8x8
//   beside it, which has not been written.
//
// Timing
//   One coefficient a cycle: a coded 4x4 block costs sixteen cycles, an
//   uncoded one one. An I_PCM macroblock costs 384.
module h264_residual (
    input  wire        clk,
    input  wire        rst_n,

    // Coefficient writes from stage A: {half, blk[4:0], pos[3:0]}.
    input  wire        cw_en,
    input  wire [9:0]  cw_addr,
    input  wire [15:0] cw_data,
    // A block's mask: {half, blk[4:0]}. Writing one marks the block coded.
    input  wire        mw_en,
    input  wire [5:0]  mw_addr,
    input  wire [15:0] mw_mask,
    // Forget every block of a half, before stage A fills it again.
    input  wire        clr_en,
    input  wire        clr_half,

    // One macroblock: the half it is in and its parameters, held from
    // `go` until `done`.
    input  wire        go,
    input  wire        half,
    input  wire [2:0]  kind,
    input  wire [5:0]  qp_y,
    input  wire [5:0]  qp_cb,
    input  wire [5:0]  qp_cr,
    output wire        busy,
    output reg         done,

    // The residual buffer, for stage B2: {half, blk[4:0], row[1:0]},
    // four ten-bit residuals of one block row, column 0 lowest. One
    // cycle from address to data.
    input  wire [7:0]  rr_addr,
    output reg  [39:0] rr_data,
    // Per half: bit `blk` set when that block's residual is all zero and
    // was not written.
    output reg  [23:0] zf0,
    output reg  [23:0] zf1
);
    localparam [2:0] K_I4  = 3'd0;
    localparam [2:0] K_I16 = 3'd1;
    localparam [2:0] K_PCM = 3'd3;

    // -----------------------------------------------------------------
    // Memories
    // -----------------------------------------------------------------
    reg [15:0] cmem [0:1023];
    reg [15:0] mmem [0:63];
    reg [39:0] rbuf [0:255];
    reg [26:0] coded0, coded1;

    always @(posedge clk) begin
        if (cw_en) cmem[cw_addr] <= cw_data;
        if (mw_en) mmem[mw_addr] <= mw_mask;
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            coded0 <= 27'd0;
            coded1 <= 27'd0;
        end else begin
            if (clr_en && !clr_half) coded0 <= 27'd0;
            else if (mw_en && !mw_addr[5]) coded0[mw_addr[4:0]] <= 1'b1;
            if (clr_en && clr_half) coded1 <= 27'd0;
            else if (mw_en && mw_addr[5]) coded1[mw_addr[4:0]] <= 1'b1;
        end
    end

    wire [26:0] coded = half ? coded1 : coded0;

    // -----------------------------------------------------------------
    // Quantiser arithmetic
    // -----------------------------------------------------------------
    // qP / 6 and qP % 6 for qP in 0..51.
    function [6:0] divmod6;
        input [5:0] q;
        reg [3:0] d;
        reg [5:0] m;
        begin
            if      (q >= 6'd48) d = 4'd8;
            else if (q >= 6'd42) d = 4'd7;
            else if (q >= 6'd36) d = 4'd6;
            else if (q >= 6'd30) d = 4'd5;
            else if (q >= 6'd24) d = 4'd4;
            else if (q >= 6'd18) d = 4'd3;
            else if (q >= 6'd12) d = 4'd2;
            else if (q >= 6'd6)  d = 4'd1;
            else                 d = 4'd0;
            m = q - 6'd6 * {2'd0, d};
            divmod6 = {d, m[2:0]};
        end
    endfunction

    // normAdjust4x4 (8-314, 8-315): row m = qP % 6; column 0 for
    // (i % 2, j % 2) = (0, 0), 1 for (1, 1), 2 otherwise.
    function [4:0] norm_adjust;
        input [2:0] m;
        input [1:0] cls;
        begin
            case ({m, cls})
                {3'd0, 2'd0}: norm_adjust = 5'd10;
                {3'd0, 2'd1}: norm_adjust = 5'd16;
                {3'd0, 2'd2}: norm_adjust = 5'd13;
                {3'd1, 2'd0}: norm_adjust = 5'd11;
                {3'd1, 2'd1}: norm_adjust = 5'd18;
                {3'd1, 2'd2}: norm_adjust = 5'd14;
                {3'd2, 2'd0}: norm_adjust = 5'd13;
                {3'd2, 2'd1}: norm_adjust = 5'd20;
                {3'd2, 2'd2}: norm_adjust = 5'd16;
                {3'd3, 2'd0}: norm_adjust = 5'd14;
                {3'd3, 2'd1}: norm_adjust = 5'd23;
                {3'd3, 2'd2}: norm_adjust = 5'd18;
                {3'd4, 2'd0}: norm_adjust = 5'd16;
                {3'd4, 2'd1}: norm_adjust = 5'd25;
                {3'd4, 2'd2}: norm_adjust = 5'd20;
                {3'd5, 2'd0}: norm_adjust = 5'd18;
                {3'd5, 2'd1}: norm_adjust = 5'd29;
                {3'd5, 2'd2}: norm_adjust = 5'd23;
                default:      norm_adjust = 5'd0;
            endcase
        end
    endfunction

    function [1:0] pos_class;
        input [3:0] pos;               // i = pos[3:2], j = pos[1:0]
        begin
            if (!pos[2] && !pos[0])     pos_class = 2'd0;
            else if (pos[2] && pos[0])  pos_class = 2'd1;
            else                        pos_class = 2'd2;
        end
    endfunction

    // weightScale4x4(i, j): Flat_4x4_16. The place for a scaling list.
    function [7:0] weight;
        input [3:0] pos;
        begin
            weight = 8'd16;
        end
    endfunction

    // 8-336 / 8-337.
    function signed [15:0] scale_ac;
        input signed [15:0] c;
        input [6:0] dm;                // {qP / 6, qP % 6}
        input [3:0] pos;
        reg        [12:0] lsu;
        reg signed [29:0] ls;
        reg signed [29:0] prod;
        reg signed [29:0] r;
        begin
            lsu  = weight(pos) * norm_adjust(dm[2:0], pos_class(pos));
            ls   = $signed({17'd0, lsu});
            prod = $signed({{14{c[15]}}, c}) * ls;
            if (dm[6:3] >= 4'd4)
                r = prod <<< (dm[6:3] - 4'd4);
            else
                r = (prod + (30'sd1 <<< (4'd3 - dm[6:3]))) >>> (4'd4 - dm[6:3]);
            scale_ac = r[15:0];
        end
    endfunction

    // LevelScale4x4(qP % 6, 0, 0).
    function [12:0] ls00;
        input [2:0] m;
        begin
            ls00 = weight(4'd0) * norm_adjust(m, 2'd0);
        end
    endfunction

    // 8-321 / 8-322: one Intra_16x16 DC value from its Hadamard result.
    function signed [15:0] scale_dcy;
        input signed [19:0] f;
        input [6:0] dm;
        reg signed [33:0] prod;
        reg signed [33:0] r;
        begin
            prod = $signed({{14{f[19]}}, f}) * $signed({21'd0, ls00(dm[2:0])});
            if (dm[6:3] >= 4'd6)
                r = prod <<< (dm[6:3] - 4'd6);
            else
                r = (prod + (34'sd1 <<< (4'd5 - dm[6:3]))) >>> (4'd6 - dm[6:3]);
            scale_dcy = r[15:0];
        end
    endfunction

    // 8-326: one chroma DC value from its 2x2 transform result.
    function signed [15:0] scale_dcc;
        input signed [19:0] f;
        input [6:0] dm;
        reg signed [33:0] prod;
        reg signed [33:0] r;
        begin
            prod = $signed({{14{f[19]}}, f}) * $signed({21'd0, ls00(dm[2:0])});
            r = (prod <<< dm[6:3]) >>> 5;
            scale_dcc = r[15:0];
        end
    endfunction

    wire [6:0] dm_y  = divmod6(qp_y);
    wire [6:0] dm_cb = divmod6(qp_cb);
    wire [6:0] dm_cr = divmod6(qp_cr);

    // -----------------------------------------------------------------
    // The walk
    // -----------------------------------------------------------------
    localparam [2:0] S_IDLE  = 3'd0;
    localparam [2:0] S_LDC   = 3'd1;   // read the luma DC block
    localparam [2:0] S_LDCW  = 3'd2;   // wait for its Hadamard
    localparam [2:0] S_CDC   = 3'd3;   // read the two chroma DC blocks
    localparam [2:0] S_CDCW  = 3'd4;   // scale them
    localparam [2:0] S_BLK   = 3'd5;   // the 24 blocks
    localparam [2:0] S_DRAIN = 3'd6;   // the transform empties
    localparam [2:0] S_PCM   = 3'd7;   // copy the samples

    reg [2:0]  state;
    reg [4:0]  blk;
    reg [3:0]  pos;
    reg [3:0]  dcrows;                 // Hadamard rows still to come back
    reg [3:0]  drain;

    // The scaled DC values: dcY in raster order of the 4x4 array (which
    // Figure 8-6 makes the raster order of the blocks themselves), and
    // dcC with Cb's four first.
    reg signed [15:0] dcy [0:15];
    reg signed [15:0] dcc [0:7];

    // Chroma DC levels as they are read.
    reg signed [15:0] cdc0, cdc1, cdc2, cdc3, cdc4, cdc5, cdc6, cdc7;

    assign busy = (state != S_IDLE);

    // luma4x4BlkIdx to the raster index of its block (6.4.3): the block
    // at x = 4 * (bx), y = 4 * (by) is dcY[by][bx].
    function [3:0] blk_raster;
        input [3:0] b;
        begin
            blk_raster = {b[3], b[1], b[2], b[0]};
        end
    endfunction

    // This block's DC, when it has one taken from elsewhere.
    wire        blk_is_chroma = (blk >= 5'd16);
    wire        blk_has_dc    = blk_is_chroma || (kind == K_I16);
    wire signed [15:0] blk_dc = blk_is_chroma ? dcc[blk[2:0]]
                                              : dcy[blk_raster(blk[3:0])];
    wire [15:0] blk_mask = mmem[{half, blk}];
    wire        blk_coded = coded[blk] && (blk_mask != 16'd0);
    wire        blk_work  = blk_coded || (blk_has_dc && blk_dc != 16'sd0);

    // Read stage: address now, data and context next cycle.
    reg         r_v;
    reg  [4:0]  r_blk;
    reg  [3:0]  r_pos;
    reg         r_bit;                 // the mask bit of this position
    reg  [1:0]  r_what;                // 0 block, 1 luma DC, 2 chroma DC, 3 PCM
    reg  [15:0] cq;

    reg         rd_v;
    reg  [9:0]  rd_a;
    reg  [1:0]  rd_what;

    always @* begin
        rd_v    = 1'b0;
        rd_a    = {half, blk, pos};
        rd_what = 2'd0;
        case (state)
            S_LDC: begin
                rd_v    = 1'b1;
                rd_a    = {half, 5'd24, pos};
                rd_what = 2'd1;
            end
            S_CDC: begin
                rd_v    = 1'b1;
                rd_a    = {half, pos[2] ? 5'd26 : 5'd25, 2'b00, pos[1:0]};
                rd_what = 2'd2;
            end
            S_BLK: begin
                rd_v    = blk_work;
                rd_what = 2'd0;
            end
            S_PCM: begin
                rd_v    = 1'b1;
                rd_what = 2'd3;
            end
            default: ;
        endcase
    end

    wire [4:0]  rd_mblk  = (state == S_LDC) ? 5'd24 :
                           (state == S_CDC) ? (pos[2] ? 5'd26 : 5'd25) : blk;
    wire [15:0] rd_mask  = mmem[{half, rd_mblk}];
    wire [3:0]  rd_mpos  = (state == S_CDC) ? {2'b00, pos[1:0]} : pos;
    wire        rd_coded = coded[rd_mblk];

    always @(posedge clk) begin
        cq <= cmem[rd_a];
    end

    // Scale stage: the transform's input.
    wire signed [15:0] lvl = r_bit ? $signed(cq) : 16'sd0;
    wire [6:0] r_dm = (r_blk >= 5'd20) ? dm_cr : ((r_blk >= 5'd16) ? dm_cb : dm_y);
    wire r_dcpos = (r_pos == 4'd0) && ((r_blk >= 5'd16) || (kind == K_I16));
    wire signed [15:0] r_dc = (r_blk >= 5'd16) ? dcc[r_blk[2:0]]
                                               : dcy[blk_raster(r_blk[3:0])];

    reg               t_v;
    reg signed [15:0] t_d;
    reg               t_had;
    reg        [5:0]  t_tag;

    wire        o_valid;
    wire [1:0]  o_row;
    wire [5:0]  o_tag;
    wire        o_had;
    wire [79:0] o_data;

    h264_itrans4 xform (
        .clk(clk), .rst_n(rst_n),
        .in_valid(t_v), .in_d(t_d), .had(t_had), .in_tag(t_tag),
        .out_valid(o_valid), .out_row(o_row), .out_tag(o_tag), .out_had(o_had),
        .out_data(o_data)
    );

    // The PCM gather: four samples make a residual word.
    reg [29:0] pcm_acc;

    // The chroma DC transform, 8-324: f = [1 1; 1 -1] c [1 1; 1 -1].
    function [79:0] dc2x2;
        input signed [15:0] c00, c01, c10, c11;
        reg signed [19:0] a00, a01, a10, a11;
        begin
            a00 = c00; a01 = c01; a10 = c10; a11 = c11;
            dc2x2 = {(a00 - a01) - (a10 - a11),   // f11
                     (a00 + a01) - (a10 + a11),   // f10
                     (a00 - a01) + (a10 - a11),   // f01
                     (a00 + a01) + (a10 + a11)};  // f00
        end
    endfunction

    wire [79:0] fcb = dc2x2(cdc0, cdc1, cdc2, cdc3);
    wire [79:0] fcr = dc2x2(cdc4, cdc5, cdc6, cdc7);
    wire [79:0] fsel = pos[2] ? fcr : fcb;
    wire [19:0] cdc_f = fsel[20 * pos[1:0] +: 20];

    integer k;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state <= S_IDLE;
            blk <= 5'd0; pos <= 4'd0; dcrows <= 4'd0; drain <= 4'd0;
            done <= 1'b0;
            r_v <= 1'b0; r_blk <= 5'd0; r_pos <= 4'd0; r_bit <= 1'b0; r_what <= 2'd0;
            t_v <= 1'b0; t_d <= 16'sd0; t_had <= 1'b0; t_tag <= 6'd0;
            pcm_acc <= 30'd0;
            zf0 <= 24'd0; zf1 <= 24'd0;
            cdc0 <= 16'sd0; cdc1 <= 16'sd0; cdc2 <= 16'sd0; cdc3 <= 16'sd0;
            cdc4 <= 16'sd0; cdc5 <= 16'sd0; cdc6 <= 16'sd0; cdc7 <= 16'sd0;
            for (k = 0; k < 16; k = k + 1) dcy[k] <= 16'sd0;
            for (k = 0; k < 8; k = k + 1) dcc[k] <= 16'sd0;
        end else begin
            done <= 1'b0;

            // ---- read stage ----
            r_v    <= rd_v;
            r_blk  <= rd_mblk;
            r_pos  <= pos;
            r_bit  <= rd_coded && rd_mask[rd_mpos];
            r_what <= rd_what;

            // ---- scale stage ----
            t_v   <= r_v && (r_what == 2'd0 || r_what == 2'd1);
            t_had <= (r_what == 2'd1);
            t_tag <= {1'b0, r_blk};
            if (r_what == 2'd1)
                t_d <= lvl;
            else
                t_d <= r_dcpos ? r_dc : scale_ac(lvl, r_dm, r_pos);

            if (r_v && r_what == 2'd2) begin
                case (r_pos[2:0])
                    3'd0: cdc0 <= lvl;
                    3'd1: cdc1 <= lvl;
                    3'd2: cdc2 <= lvl;
                    3'd3: cdc3 <= lvl;
                    3'd4: cdc4 <= lvl;
                    3'd5: cdc5 <= lvl;
                    3'd6: cdc6 <= lvl;
                    default: cdc7 <= lvl;
                endcase
            end

            // ---- transform output ----
            if (o_valid && o_had) begin
                dcy[{o_row, 2'd0}] <= scale_dcy(o_data[19:0],  dm_y);
                dcy[{o_row, 2'd1}] <= scale_dcy(o_data[39:20], dm_y);
                dcy[{o_row, 2'd2}] <= scale_dcy(o_data[59:40], dm_y);
                dcy[{o_row, 2'd3}] <= scale_dcy(o_data[79:60], dm_y);
            end

            // PCM samples: four to a word, written below.
            if (r_v && r_what == 2'd3)
                pcm_acc <= {2'b00, cq[7:0], pcm_acc[29:10]};

            // ---- the walk ----
            case (state)
                S_IDLE: if (go) begin
                    blk <= 5'd0;
                    pos <= 4'd0;
                    if (kind == K_PCM) begin
                        state <= S_PCM;
                    end else if (kind == K_I16 && coded[24]) begin
                        state  <= S_LDC;
                        dcrows <= 4'd4;
                    end else begin
                        for (k = 0; k < 16; k = k + 1) dcy[k] <= 16'sd0;
                        state <= S_CDC;
                    end
                end
                S_LDC: begin
                    pos <= pos + 4'd1;
                    if (pos == 4'd15) state <= S_LDCW;
                end
                S_LDCW: begin
                    if (o_valid && o_had && o_row == 2'd3) begin
                        pos   <= 4'd0;
                        state <= S_CDC;
                    end
                end
                S_CDC: begin
                    pos <= pos + 4'd1;
                    if (pos == 4'd7) begin
                        pos   <= 4'd0;
                        drain <= 4'd2;
                        state <= S_CDCW;
                    end
                end
                S_CDCW: begin
                    // Two cycles for the last level to land in cdc7.
                    if (drain != 4'd0) begin
                        drain <= drain - 4'd1;
                    end else begin
                        // One a cycle, through one multiplier: Cb's four,
                        // then Cr's.
                        dcc[pos[2:0]] <= scale_dcc(cdc_f, pos[2] ? dm_cr : dm_cb);
                        pos <= pos + 4'd1;
                        if (pos == 4'd7) begin
                            blk   <= 5'd0;
                            pos   <= 4'd0;
                            state <= S_BLK;
                        end
                    end
                end
                S_BLK: begin
                    if (blk_work) begin
                        if (half) zf1[blk] <= 1'b0; else zf0[blk] <= 1'b0;
                        pos <= pos + 4'd1;
                        if (pos == 4'd15) begin
                            if (blk == 5'd23) begin
                                drain <= 4'd8;
                                state <= S_DRAIN;
                            end
                            blk <= blk + 5'd1;
                        end
                    end else begin
                        if (half) zf1[blk] <= 1'b1; else zf0[blk] <= 1'b1;
                        pos <= 4'd0;
                        if (blk == 5'd23) begin
                            drain <= 4'd8;
                            state <= S_DRAIN;
                        end
                        blk <= blk + 5'd1;
                    end
                end
                S_PCM: begin
                    if (half) zf1[blk] <= 1'b0; else zf0[blk] <= 1'b0;
                    pos <= pos + 4'd1;
                    if (pos == 4'd15) begin
                        if (blk == 5'd23) begin
                            drain <= 4'd3;
                            state <= S_DRAIN;
                        end
                        blk <= blk + 5'd1;
                    end
                end
                S_DRAIN: begin
                    if (drain != 4'd0) drain <= drain - 4'd1;
                    else begin
                        state <= S_IDLE;
                        done  <= 1'b1;
                    end
                end
                default: state <= S_IDLE;
            endcase
        end
    end

    // The residual buffer: transform rows, or gathered PCM samples.
    wire        pcm_w   = r_v && (r_what == 2'd3) && (r_pos[1:0] == 2'd3);
    wire [39:0] pcm_word = {2'b00, cq[7:0], pcm_acc[29:0]};
    wire [39:0] xf_word  = {o_data[69:60], o_data[49:40], o_data[29:20], o_data[9:0]};

    // One write port, so the buffer is one block RAM.
    wire        rb_we = pcm_w || (o_valid && !o_had);
    wire [7:0]  rb_wa = pcm_w ? {half, r_blk, r_pos[3:2]} : {half, o_tag[4:0], o_row};
    wire [39:0] rb_wd = pcm_w ? pcm_word : xf_word;

    always @(posedge clk) begin
        if (rb_we) rbuf[rb_wa] <= rb_wd;
        rr_data <= rbuf[rr_addr];
    end
endmodule
