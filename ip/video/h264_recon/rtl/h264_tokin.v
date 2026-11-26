// h264_tokin — stage A of `h264_recon`: the syntax token stream in,
// one macroblock's worth of work out.
//
// What it does
//   Reads the tokens of `docs/h264-decoder.md` §1 (after `h264_mvpred`
//   has added the `MV`s) and, for each macroblock:
//   - writes its coefficient levels into a half of `h264_residual`'s
//     coefficient memory at their raster positions — the inverse
//     zig-zag of 8.5.6 (Table 8-13) — with a mask per block, or its
//     I_PCM samples;
//   - keeps what the deblocking filter's boundary strengths depend on
//     (8.7.2.1): whether each macroblock is intra, which 4x4 luma
//     blocks have non-zero coefficients, and every 4x4 block's motion
//     vector and reference — for this macroblock, the right column of
//     the one to its left, and in a line buffer the bottom row of every
//     macroblock of the row above;
//   - and on `MB_END` derives the 32 boundary strengths of its luma
//     edges, the availability of its neighbours for intra prediction,
//     the deblocking flags and the QPs, and writes them as one
//     descriptor into slot `a_cnt % 8` of the descriptor ring (`d_*`)
//     that the three later stages read.
//
// What is resolved here
//   - Chroma QP (8.5.8, Table 8-15) from QPY and chroma_qp_index_offset
//     (Cb) or second_chroma_qp_index_offset (Cr).
//   - mbAddrA..D availability (6.4.9 for a frame without ASO: a
//     neighbour exists and its address is not below first_mb_in_slice),
//     and for intra prediction constrained_intra_pred_flag's rule that
//     an inter neighbour does not count (8.3.1.2 step 3, 8.3.3, 8.3.4).
//   - filterLeftMbEdgeFlag, filterTopMbEdgeFlag, filterInternalEdgesFlag
//     (8.7 step 2).
//   - bS, 8.7.2.1 for frames: 4 on a macroblock edge with an intra
//     macroblock on either side (I_PCM is intra), 3 inside an intra
//     macroblock, 2 where either 4x4 block has non-zero coefficients
//     (a LumaLevel4x4 block with TotalCoeff > 0), 1 where the two blocks'
//     reference or either motion vector component differs by 4 quarter
//     samples or more, else 0.
//
// What is not
//   - "different reference pictures" is decided on ref_idx, which is the
//     same thing inside a slice whose list holds no picture twice. A
//     list that holds a picture twice (reordering can do that), or two
//     slices of one picture with different lists, would need the
//     picture each index names; the DPB manager has it and this does
//     not.
//   - B macroblocks (two motion vectors), 8x8 transforms, SI/SP.
//   - Errors: an `ERROR` token inside a macroblock ends it as if it were
//     complete, so the later stages stay in step. Concealment is not
//     done.
//
// The `MV` token is `h264_mvpred`'s (docs/h264-decoder.md): [23] list,
// [22:19] luma4x4BlkIdx, [18] component; horizontal [15:0], of which the
// 14 bits Table A-1 allows are kept; vertical [17:13] refIdxL0 and
// [12:0]. Only list 0 is read.
module h264_tokin #(
    parameter integer MAX_W_MBS = 80
) (
    input  wire         clk,
    input  wire         rst_n,

    input  wire         tok_valid,
    output wire         tok_ready,
    input  wire [31:0]  tok,

    // The frame buffer the picture starting now goes to.
    input  wire [3:0]   frame_idx,

    // Pipeline position: how many macroblocks this stage has handed on,
    // and how many the residual stage and the deblocker have finished
    // (all modulo 8).
    output reg  [2:0]   a_cnt,
    input  wire [2:0]   b1_cnt,
    input  wire [2:0]   c_cnt,

    // Coefficient memory writes (`h264_residual`).
    output reg          cw_en,
    output reg  [9:0]   cw_addr,
    output reg  [15:0]  cw_data,
    output reg          mw_en,
    output reg  [5:0]   mw_addr,
    output reg  [15:0]  mw_mask,
    output reg          clr_en,
    output reg          clr_half,

    // The descriptor of macroblock a_cnt, written on `d_we`.
    output reg          d_we,
    output reg  [2:0]   d_idx,
    // For h264_residual.
    output reg  [2:0]   d_kind,
    output reg  [5:0]   d_qp_y,
    output reg  [5:0]   d_qp_cb,
    output reg  [5:0]   d_qp_cr,
    // For h264_intra.
    output reg  [6:0]   d_mb_x,
    output reg  [1:0]   d_i16_mode,
    output reg  [1:0]   d_c_mode,
    output reg  [63:0]  d_i4_modes,
    output reg  [3:0]   d_avail,          // {D, C, B, A}
    // For h264_deblock.
    output reg  [6:0]   d_mb_y,
    output reg          d_last_col,
    output reg          d_last_row,
    output reg  [3:0]   d_frame,
    output reg  [2:0]   d_filt,           // {inner, top, left}
    output reg          d_t8x8,
    output reg  [95:0]  d_bs,
    output reg  [53:0]  d_qps,            // 9 QPs: y, y_left, y_top, cb, cb_left, cb_top, cr, ...
    output reg  [4:0]   d_off_a,
    output reg  [4:0]   d_off_b
);
    // -----------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------
    localparam [7:0] T_SPS_DIMS   = 8'h01;
    localparam [7:0] T_SLICE_HDR  = 8'h02;
    localparam [7:0] T_SLICE_QP   = 8'h03;
    localparam [7:0] T_SLICE_DBLK = 8'h04;
    localparam [7:0] T_SLICE_FIRST= 8'h06;
    localparam [7:0] T_MB_START   = 8'h10;
    localparam [7:0] T_MB_TYPE    = 8'h11;
    localparam [7:0] T_MB_QP      = 8'h12;
    localparam [7:0] T_I4_MODE    = 8'h13;
    localparam [7:0] T_MV         = 8'h16;
    localparam [7:0] T_MB_CBP     = 8'h19;
    localparam [7:0] T_COEF_BLK   = 8'h20;
    localparam [7:0] T_COEF       = 8'h21;
    localparam [7:0] T_PCM        = 8'h30;
    localparam [7:0] T_MB_END     = 8'h3F;
    localparam [7:0] T_ERROR      = 8'h7F;

    localparam [2:0] K_I4  = 3'd0;
    localparam [2:0] K_I16 = 3'd1;
    localparam [2:0] K_INT = 3'd2;
    localparam [2:0] K_PCM = 3'd3;

    wire [7:0]  tag  = tok[31:24];
    wire [23:0] data = tok[23:0];

    // -----------------------------------------------------------------
    // Picture and slice state
    // -----------------------------------------------------------------
    reg [6:0]  w_mbs, h_mbs;
    reg [12:0] first_mb;
    reg        cip;
    reg [1:0]  idc;
    reg signed [4:0] off_a, off_b;
    reg signed [4:0] cqp_off, cqp_off2;
    reg [3:0]  frame;

    // A signed eight-bit offset held to -12..12, which is all 7.4.2.2
    // allows and five bits can hold.
    function signed [4:0] off5;
        input [7:0] v;
        begin
            if ($signed(v) > 8'sd12)       off5 = 5'sd12;
            else if ($signed(v) < -8'sd12) off5 = -5'sd12;
            else                           off5 = v[4:0];
        end
    endfunction

    // -----------------------------------------------------------------
    // The macroblock
    // -----------------------------------------------------------------
    reg [6:0]  mb_x, mb_y;
    reg [2:0]  kind;
    reg [1:0]  i16_mode, c_mode;
    reg [63:0] i4_modes;
    reg [5:0]  qp;
    reg        t8x8;
    reg [15:0] nz;                  // luma4x4BlkIdx with TotalCoeff > 0
    reg [13:0] mvx [0:15];
    reg [13:0] mvy [0:15];
    reg [4:0]  mref [0:15];

    // The block whose levels are arriving.
    reg        blk_open;
    reg [4:0]  blk;
    reg        blk_zz;              // its positions go through the zig-zag
    reg [15:0] mask;
    reg [8:0]  pcm_n;               // I_PCM samples written
    reg [1:0]  pcm_k;               // sample of the token being written

    wire half = a_cnt[0];

    // Table 8-13's zig-zag: scan index -> raster position 4i + j.
    function [3:0] zigzag;
        input [3:0] idx;
        begin
            case (idx)
                4'd0:  zigzag = 4'd0;
                4'd1:  zigzag = 4'd1;
                4'd2:  zigzag = 4'd4;
                4'd3:  zigzag = 4'd8;
                4'd4:  zigzag = 4'd5;
                4'd5:  zigzag = 4'd2;
                4'd6:  zigzag = 4'd3;
                4'd7:  zigzag = 4'd6;
                4'd8:  zigzag = 4'd9;
                4'd9:  zigzag = 4'd12;
                4'd10: zigzag = 4'd13;
                4'd11: zigzag = 4'd10;
                4'd12: zigzag = 4'd7;
                4'd13: zigzag = 4'd11;
                4'd14: zigzag = 4'd14;
                default: zigzag = 4'd15;
            endcase
        end
    endfunction

    // An I_PCM sample's block and position: luma raster 0..255, then
    // Cb 0..63, Cr 0..63 (8.3.5).
    function [8:0] pcm_place;            // {blk[4:0], pos[3:0]}
        input [8:0] i;
        reg [3:0] x, y;
        reg [2:0] cx, cy;
        begin
            if (!i[8]) begin
                x = i[3:0];
                y = i[7:4];
                pcm_place = {1'b0, y[3], x[3], y[2], x[2], y[1:0], x[1:0]};
            end else begin
                cx = i[2:0];
                cy = i[5:3];
                pcm_place = {2'b10, i[6], cy[2], cx[2], cy[1:0], cx[1:0]};
            end
        end
    endfunction

    // -----------------------------------------------------------------
    // Neighbours
    // -----------------------------------------------------------------
    // The left macroblock.
    reg        l_intra;
    reg [5:0]  l_qp;                // QPY as the deblocking filter takes it
    reg [3:0]  l_nz;                // right column: blocks (3, k)
    reg [13:0] l_mvx [0:3];
    reg [13:0] l_mvy [0:3];
    reg [4:0]  l_ref [0:3];

    // The row above: one entry per macroblock column.
    reg [79:0] row_intra;
    reg        ul_intra, ul_next;   // the above-left's, saved before it is overwritten
    // One block RAM: entry 4x + k is {qp[5:0], nz bottom row[3:0]} of
    // the macroblock (written into all four) and {ref, mvy, mvx} of its
    // block (k, 3).
    reg [42:0] tmv   [0:4*MAX_W_MBS-1];
    reg [9:0]  t_info_q;
    reg [42:0] tmv_q;
    reg [8:0]  tmv_ra;
    reg        tmv_we;
    reg [8:0]  tmv_wa;
    reg [42:0] tmv_wd;
    reg [13:0] t_mvx [0:3];
    reg [13:0] t_mvy [0:3];
    reg [4:0]  t_ref [0:3];

    integer n;
    initial begin
        for (n = 0; n < 4 * MAX_W_MBS; n = n + 1) tmv[n] = 43'd0;
    end

    always @(posedge clk) begin
        if (tmv_we) tmv[tmv_wa] <= tmv_wd;
        tmv_q <= tmv[tmv_ra];
    end

    // -----------------------------------------------------------------
    // Table 8-15
    // -----------------------------------------------------------------
    function [5:0] qpc;
        input [5:0] qpy;
        input signed [4:0] off;
        reg signed [7:0] qpi;
        begin
            qpi = $signed({2'b00, qpy}) + {{3{off[4]}}, off};
            if (qpi < 8'sd0) qpi = 8'sd0;
            if (qpi > 8'sd51) qpi = 8'sd51;
            if (qpi < 8'sd30) qpc = qpi[5:0];
            else begin
                case (qpi[5:0])
                    6'd30: qpc = 6'd29;
                    6'd31: qpc = 6'd30;
                    6'd32: qpc = 6'd31;
                    6'd33: qpc = 6'd32;
                    6'd34: qpc = 6'd32;
                    6'd35: qpc = 6'd33;
                    6'd36: qpc = 6'd34;
                    6'd37: qpc = 6'd34;
                    6'd38: qpc = 6'd35;
                    6'd39: qpc = 6'd35;
                    6'd40: qpc = 6'd36;
                    6'd41: qpc = 6'd36;
                    6'd42: qpc = 6'd37;
                    6'd43: qpc = 6'd37;
                    6'd44: qpc = 6'd37;
                    6'd45: qpc = 6'd38;
                    6'd46: qpc = 6'd38;
                    6'd47: qpc = 6'd38;
                    default: qpc = 6'd39;
                endcase
            end
        end
    endfunction

    // -----------------------------------------------------------------
    // Control
    // -----------------------------------------------------------------
    localparam [2:0] S_TOK   = 3'd0;   // between macroblocks, or inside one
    localparam [2:0] S_WAIT  = 3'd1;   // MB_START seen: wait for room
    localparam [2:0] S_PCM   = 3'd2;   // writing a PCM token's three samples
    localparam [2:0] S_TOP   = 3'd3;   // MB_END: read the row above
    localparam [2:0] S_BS    = 3'd4;   // the 32 boundary strengths
    localparam [2:0] S_DESC  = 3'd5;   // descriptor, line buffer, hand-off

    reg [2:0]  st;
    reg        in_mb;
    reg [5:0]  step;
    reg [23:0] pcm_data;
    reg [95:0] bs;
    reg [12:0] addr;                   // CurrMbAddr

    wire [2:0] a_ahead_b1 = a_cnt - b1_cnt;
    wire [2:0] a_ahead_c  = a_cnt - c_cnt;
    // Room for macroblock a_cnt: its coefficient half is free once the
    // residual stage has finished a_cnt - 2, and its descriptor slot once
    // the deblocker has finished a_cnt - 8.
    wire room = (a_ahead_b1 <= 3'd1) && (a_ahead_c <= 3'd6);

    assign tok_ready = (st == S_TOK);
    wire take = tok_valid && tok_ready;

    // luma4x4BlkIdx of the 4x4 block at (x, y), in blocks.
    function [3:0] blk_at;
        input [1:0] x;
        input [1:0] y;
        begin
            blk_at = {y[1], x[1], y[0], x[0]};
        end
    endfunction

    // -----------------------------------------------------------------
    // One boundary strength (8.7.2.1), segment `s` = step
    // -----------------------------------------------------------------
    wire        sv    = !step[4];                 // vertical edges first
    wire [1:0]  se    = step[3:2];
    wire [1:0]  sk    = step[1:0];
    wire [3:0]  q_blk = sv ? blk_at(se, sk) : blk_at(sk, se);
    wire [3:0]  p_blk = sv ? blk_at(se - 2'd1, sk) : blk_at(sk, se - 2'd1);
    wire        s_mb  = (se == 2'd0);             // a macroblock edge
    wire        cur_intra = (kind != K_INT);
    wire        p_intra = s_mb ? (sv ? l_intra : row_intra[mb_x]) : cur_intra;
    wire        p_nz    = s_mb ? (sv ? l_nz[sk] : t_info_q[sk]) : nz[p_blk];
    wire [13:0] p_mvx   = s_mb ? (sv ? l_mvx[sk] : t_mvx[sk]) : mvx[p_blk];
    wire [13:0] p_mvy   = s_mb ? (sv ? l_mvy[sk] : t_mvy[sk]) : mvy[p_blk];
    wire [4:0]  p_ref   = s_mb ? (sv ? l_ref[sk] : t_ref[sk]) : mref[p_blk];
    wire [14:0] dmx = {p_mvx[13], p_mvx} - {mvx[q_blk][13], mvx[q_blk]};
    wire [14:0] dmy = {p_mvy[13], p_mvy} - {mvy[q_blk][13], mvy[q_blk]};
    wire [14:0] amx = dmx[14] ? (~dmx + 15'd1) : dmx;
    wire [14:0] amy = dmy[14] ? (~dmy + 15'd1) : dmy;
    wire [2:0]  s_bs = (p_intra || cur_intra) ? (s_mb ? 3'd4 : 3'd3) :
                       (p_nz || nz[q_blk]) ? 3'd2 :
                       ((p_ref != mref[q_blk]) || (amx >= 15'd4) || (amy >= 15'd4)) ? 3'd1 : 3'd0;

    // CurrMbAddr and its neighbours' addresses, as the tests of 6.4.9.
    wire [12:0] w13 = {6'd0, w_mbs};
    wire a_in = (mb_x != 7'd0) && (addr - 13'd1 >= first_mb);
    wire b_in = (mb_y != 7'd0) && (addr - w13 >= first_mb);
    wire c_in = (mb_y != 7'd0) && (mb_x + 7'd1 != w_mbs) && (addr - w13 + 13'd1 >= first_mb);
    wire d_in = (mb_x != 7'd0) && (mb_y != 7'd0) && (addr - w13 - 13'd1 >= first_mb);
    // constrained_intra_pred_flag: an inter neighbour of an intra
    // macroblock is not available to it.
    wire ci = cip && cur_intra;
    wire av_a = a_in && !(ci && !l_intra);
    wire av_b = b_in && !(ci && !row_intra[mb_x]);
    wire av_c = c_in && !(ci && !row_intra[mb_x + 7'd1]);
    wire av_d = d_in && !(ci && !ul_intra);

    wire [5:0] qp_dbf = (kind == K_PCM) ? 6'd0 : qp;
    wire [5:0] top_qp = t_info_q[9:4];

    integer i;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            st <= S_TOK; in_mb <= 1'b0; step <= 6'd0;
            a_cnt <= 3'd0;
            w_mbs <= 7'd1; h_mbs <= 7'd1; first_mb <= 13'd0; cip <= 1'b0; idc <= 2'd0;
            off_a <= 5'sd0; off_b <= 5'sd0; cqp_off <= 5'sd0; cqp_off2 <= 5'sd0; frame <= 4'd0;
            mb_x <= 7'd0; mb_y <= 7'd0; kind <= K_I4; i16_mode <= 2'd0; c_mode <= 2'd0;
            i4_modes <= 64'd0; qp <= 6'd0; t8x8 <= 1'b0; nz <= 16'd0;
            blk_open <= 1'b0; blk <= 5'd0; blk_zz <= 1'b0; mask <= 16'd0;
            pcm_n <= 9'd0; pcm_k <= 2'd0; pcm_data <= 24'd0;
            bs <= 96'd0; addr <= 13'd0;
            l_intra <= 1'b0; l_qp <= 6'd0; l_nz <= 4'd0;
            row_intra <= 80'd0; ul_intra <= 1'b0; ul_next <= 1'b0;
            t_info_q <= 10'd0; tmv_ra <= 9'd0; tmv_we <= 1'b0; tmv_wa <= 9'd0; tmv_wd <= 43'd0;
            cw_en <= 1'b0; cw_addr <= 10'd0; cw_data <= 16'd0;
            mw_en <= 1'b0; mw_addr <= 6'd0; mw_mask <= 16'd0;
            clr_en <= 1'b0; clr_half <= 1'b0;
            d_we <= 1'b0; d_idx <= 3'd0; d_kind <= 3'd0; d_qp_y <= 6'd0; d_qp_cb <= 6'd0;
            d_qp_cr <= 6'd0; d_mb_x <= 7'd0; d_i16_mode <= 2'd0; d_c_mode <= 2'd0;
            d_i4_modes <= 64'd0; d_avail <= 4'd0; d_mb_y <= 7'd0; d_last_col <= 1'b0;
            d_last_row <= 1'b0; d_frame <= 4'd0; d_filt <= 3'd0; d_t8x8 <= 1'b0;
            d_bs <= 96'd0; d_qps <= 54'd0; d_off_a <= 5'd0; d_off_b <= 5'd0;
            for (i = 0; i < 16; i = i + 1) begin
                mvx[i] <= 14'd0; mvy[i] <= 14'd0; mref[i] <= 5'd0;
            end
            for (i = 0; i < 4; i = i + 1) begin
                l_mvx[i] <= 14'd0; l_mvy[i] <= 14'd0; l_ref[i] <= 5'd0;
                t_mvx[i] <= 14'd0; t_mvy[i] <= 14'd0; t_ref[i] <= 5'd0;
            end
        end else begin
            cw_en  <= 1'b0;
            mw_en  <= 1'b0;
            clr_en <= 1'b0;
            d_we   <= 1'b0;
            tmv_we <= 1'b0;

            case (st)
                S_TOK: if (take) begin
                    case (tag)
                        T_SPS_DIMS: begin
                            w_mbs <= data[18:12];
                            h_mbs <= data[6:0];
                        end
                        T_SLICE_HDR: cip <= data[15];
                        T_SLICE_FIRST: begin
                            first_mb <= data[12:0];
                            if (data[23:0] == 24'd0) frame <= frame_idx;
                        end
                        T_SLICE_QP: begin
                            cqp_off  <= off5(data[15:8]);
                            cqp_off2 <= off5(data[7:0]);
                        end
                        T_SLICE_DBLK: begin
                            idc   <= data[23:22];
                            off_a <= off5(data[15:8]);
                            off_b <= off5(data[7:0]);
                        end
                        T_MB_START: begin
                            mb_x <= data[18:12];
                            mb_y <= data[6:0];
                            st   <= S_WAIT;
                        end
                        T_MB_TYPE: begin
                            case (data[23:19])
                                5'd0:    kind <= K_I4;
                                5'd1:    kind <= K_I16;
                                5'd2:    kind <= K_PCM;
                                default: kind <= K_INT;
                            endcase
                            i16_mode <= data[14:13];
                        end
                        T_I4_MODE: i4_modes[4 * data[7:4] +: 4] <= data[3:0];
                        // List 0 only: list 1 is B's.
                        T_MV: if (!data[23]) begin
                            if (data[18]) begin
                                mvy[data[22:19]]  <= {data[12], data[12:0]};
                                mref[data[22:19]] <= data[17:13];
                            end else begin
                                mvx[data[22:19]] <= data[13:0];
                            end
                        end
                        T_MB_CBP: begin
                            t8x8   <= data[8];
                            c_mode <= data[7:6];
                        end
                        T_MB_QP: qp <= data[5:0];
                        T_COEF_BLK: begin
                            // The last block's mask goes first.
                            if (blk_open) begin
                                mw_en   <= 1'b1;
                                mw_addr <= {half, blk};
                                mw_mask <= mask;
                            end
                            blk_open <= 1'b1;
                            mask     <= 16'd0;
                            blk_zz   <= (data[23:20] != 4'd3) && (data[23:20] != 4'd4);
                            case (data[23:20])
                                4'd0: blk <= 5'd24;
                                4'd1: blk <= {1'b0, data[19:16]};
                                4'd2: begin
                                    blk <= {1'b0, data[19:16]};
                                    if (data[4:0] != 5'd0) nz[data[19:16]] <= 1'b1;
                                end
                                4'd3: blk <= 5'd25;
                                4'd4: blk <= 5'd26;
                                4'd5: blk <= {3'b100, data[17:16]};
                                default: blk <= {3'b101, data[17:16]};
                            endcase
                        end
                        T_COEF: begin
                            cw_en   <= 1'b1;
                            cw_addr <= {half, blk, blk_zz ? zigzag(data[21:18]) : data[21:18]};
                            cw_data <= data[15:0];
                            mask[blk_zz ? zigzag(data[21:18]) : data[21:18]] <= 1'b1;
                        end
                        T_PCM: begin
                            pcm_data <= data;
                            pcm_k    <= 2'd0;
                            st       <= S_PCM;
                        end
                        T_MB_END, T_ERROR: begin
                            if (in_mb) begin
                                if (blk_open) begin
                                    mw_en   <= 1'b1;
                                    mw_addr <= {half, blk};
                                    mw_mask <= mask;
                                end
                                blk_open <= 1'b0;
                                st       <= S_TOP;
                                step     <= 6'd0;
                                tmv_ra   <= {mb_x, 2'd0};
                            end
                        end
                        default: ;
                    endcase
                end

                // A macroblock starts when its half and its descriptor
                // slot are free; until then its first token waits.
                S_WAIT: if (room) begin
                    clr_en   <= 1'b1;
                    clr_half <= half;
                    in_mb    <= 1'b1;
                    nz       <= 16'd0;
                    blk_open <= 1'b0;
                    pcm_n    <= 9'd0;
                    t8x8     <= 1'b0;
                    c_mode   <= 2'd0;
                    i16_mode <= 2'd0;
                    addr     <= {6'd0, mb_y} * w13 + {6'd0, mb_x};
                    for (i = 0; i < 16; i = i + 1) begin
                        mvx[i] <= 14'd0; mvy[i] <= 14'd0; mref[i] <= 5'd0;
                    end
                    st <= S_TOK;
                end

                S_PCM: begin
                    cw_en   <= 1'b1;
                    cw_addr <= {half, pcm_place(pcm_n)};
                    cw_data <= {8'd0, pcm_data[8 * pcm_k +: 8]};
                    pcm_n   <= pcm_n + 9'd1;
                    pcm_k   <= pcm_k + 2'd1;
                    if (pcm_k == 2'd2) st <= S_TOK;
                end

                // The row above's entry for this column: four motion
                // vector reads, one a cycle, and the info word.
                S_TOP: begin
                    step   <= step + 6'd1;
                    tmv_ra <= {mb_x, step[1:0] + 2'd1};
                    if (step == 6'd1) t_info_q <= tmv_q[42:33];
                    if (step != 6'd0) begin
                        t_mvx[step[1:0] - 2'd1] <= tmv_q[13:0];
                        t_mvy[step[1:0] - 2'd1] <= tmv_q[27:14];
                        t_ref[step[1:0] - 2'd1] <= tmv_q[32:28];
                    end
                    if (step == 6'd4) begin
                        st   <= S_BS;
                        step <= 6'd0;
                    end
                end

                S_BS: begin
                    bs[3 * step[4:0] +: 3] <= s_bs;
                    step <= step + 6'd1;
                    if (step == 6'd31) begin
                        st   <= S_DESC;
                        step <= 6'd0;
                    end
                end

                // Hand the macroblock on, and remember it for the next.
                S_DESC: begin
                    step <= step + 6'd1;
                    if (step == 6'd0) begin
                        d_we       <= 1'b1;
                        d_idx      <= a_cnt;
                        d_kind     <= kind;
                        d_qp_y     <= qp;
                        d_qp_cb    <= qpc(qp, cqp_off);
                        d_qp_cr    <= qpc(qp, cqp_off2);
                        d_mb_x     <= mb_x;
                        d_i16_mode <= i16_mode;
                        d_c_mode   <= c_mode;
                        d_i4_modes <= i4_modes;
                        d_avail    <= {av_d, av_c, av_b, av_a};
                        d_mb_y     <= mb_y;
                        d_last_col <= (mb_x + 7'd1 == w_mbs);
                        d_last_row <= (mb_y + 7'd1 == h_mbs);
                        d_frame    <= frame;
                        d_filt     <= {idc != 2'd1,
                                       (mb_y != 7'd0) && (idc != 2'd1) && !(idc == 2'd2 && !b_in),
                                       (mb_x != 7'd0) && (idc != 2'd1) && !(idc == 2'd2 && !a_in)};
                        d_t8x8     <= t8x8;
                        d_bs       <= bs;
                        d_qps      <= {qpc(top_qp, cqp_off2), qpc(l_qp, cqp_off2),
                                       qpc(qp_dbf, cqp_off2),
                                       qpc(top_qp, cqp_off), qpc(l_qp, cqp_off), qpc(qp_dbf, cqp_off),
                                       top_qp, l_qp, qp_dbf};
                        d_off_a    <= off_a;
                        d_off_b    <= off_b;
                        // The row above's entries for this column become
                        // this macroblock's.
                        ul_intra  <= row_intra[mb_x];
                        row_intra[mb_x] <= cur_intra;
                        l_intra   <= cur_intra;
                        l_qp      <= qp_dbf;
                        l_nz      <= {nz[15], nz[13], nz[7], nz[5]};
                    end
                    // Bottom row (blocks 10, 11, 14, 15) to the line buffer,
                    // right column (5, 7, 13, 15) to the left registers.
                    if (step < 6'd4) begin
                        tmv_we <= 1'b1;
                        tmv_wa <= {mb_x, step[1:0]};
                        tmv_wd <= {qp_dbf, nz[15], nz[14], nz[11], nz[10],
                                   mref[blk_at(step[1:0], 2'd3)], mvy[blk_at(step[1:0], 2'd3)],
                                   mvx[blk_at(step[1:0], 2'd3)]};
                        l_mvx[step[1:0]] <= mvx[blk_at(2'd3, step[1:0])];
                        l_mvy[step[1:0]] <= mvy[blk_at(2'd3, step[1:0])];
                        l_ref[step[1:0]] <= mref[blk_at(2'd3, step[1:0])];
                    end
                    if (step == 6'd3) begin
                        a_cnt <= a_cnt + 3'd1;
                        in_mb <= 1'b0;
                        st    <= S_TOK;
                        step  <= 6'd0;
                    end
                end

                default: st <= S_TOK;
            endcase
        end
    end
endmodule
