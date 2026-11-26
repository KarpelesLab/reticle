// h264_recon — the reconstruction back end of the H.264 decoder: the
// syntax token stream and the inter prediction in, the decoded picture
// out to SDRAM.
//
// What it does
//   Everything ITU-T H.264 does to a macroblock after its syntax has
//   been parsed and its motion vectors found: dequantisation and the
//   inverse transforms (§8.5), intra prediction (§8.3), adding the
//   residual to the prediction, the deblocking filter (§8.7), and
//   writing the result in the tiled frame layout through one
//   `sdram_burst` client port. `docs/h264-decoder.md` has the token
//   format; `ip/video/h264_recon/README.md` has what each stage does,
//   what is verified and how.
//
// Five stages, a macroblock apart
//   A  h264_tokin     tokens -> coefficient levels, boundary strengths,
//                     neighbour availability: one descriptor a macroblock
//   B1 h264_residual  levels -> residuals            (two halves)
//   B2 h264_intra     prediction + residual -> slot  (four slots)
//   C  h264_deblock   slot -> filtered rows
//   D  h264_frame_wr  rows -> bursts
//   Each of A, B1, B2, C counts the macroblocks it has finished (modulo
//   8), and a stage starts macroblock m when the stage before it has
//   finished m and the buffer m goes into is free:
//     A  fills coefficient half m % 2   once B1 has finished m - 2;
//     B1 fills residual half m % 2      once B2 has finished m - 2;
//     B2 fills slot m % 4               once C has finished m - 3 (C uses
//                                       slot m - 1 as the left one, and
//                                       sends it, while it works on m);
//     C  works on slot m % 4.
//   So up to seven macroblocks are in flight and a slow stage only
//   stalls the ones before it.
//
// `frame_idx` names the frame buffer a picture is written to. It is
// sampled at the token that starts the picture (SLICE_FIRST with
// first_mb_in_slice 0) and travels with the picture's macroblocks;
// `pic_done` pulses when the picture's last burst has been accepted.
// Choosing the frame is the DPB manager's job, which is outside this.
module h264_recon #(
    parameter integer MAX_W_MBS = 80
) (
    input  wire        clk,
    input  wire        rst_n,

    // Syntax tokens, after h264_mvpred.
    input  wire        tok_valid,
    output wire        tok_ready,
    input  wire [31:0] tok,

    // Inter prediction samples, from h264_inter.
    input  wire        pred_valid,
    output wire        pred_ready,
    input  wire [31:0] pred_data,

    input  wire [3:0]  frame_idx,
    output wire        pic_done,

    // An sdram_burst client port. Only writes are made.
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
    input  wire        rd_last
);
    // -----------------------------------------------------------------
    // Stage A
    // -----------------------------------------------------------------
    wire [2:0]  a_cnt;
    reg  [2:0]  b1_cnt, b2_cnt, c_cnt;

    wire        cw_en, mw_en, clr_en, clr_half;
    wire [9:0]  cw_addr;
    wire [15:0] cw_data;
    wire [5:0]  mw_addr;
    wire [15:0] mw_mask;

    wire        d_we;
    wire [2:0]  d_idx, d_kind;
    wire [5:0]  d_qp_y, d_qp_cb, d_qp_cr;
    wire [6:0]  d_mb_x, d_mb_y;
    wire [1:0]  d_i16_mode, d_c_mode;
    wire [63:0] d_i4_modes;
    wire [3:0]  d_avail, d_frame;
    wire        d_last_col, d_last_row, d_t8x8;
    wire [2:0]  d_filt;
    wire [95:0] d_bs;
    wire [53:0] d_qps;
    wire [4:0]  d_off_a, d_off_b;

    h264_tokin #(.MAX_W_MBS(MAX_W_MBS)) tokin (
        .clk(clk), .rst_n(rst_n),
        .tok_valid(tok_valid), .tok_ready(tok_ready), .tok(tok),
        .frame_idx(frame_idx),
        .a_cnt(a_cnt), .b1_cnt(b1_cnt), .c_cnt(c_cnt),
        .cw_en(cw_en), .cw_addr(cw_addr), .cw_data(cw_data),
        .mw_en(mw_en), .mw_addr(mw_addr), .mw_mask(mw_mask),
        .clr_en(clr_en), .clr_half(clr_half),
        .d_we(d_we), .d_idx(d_idx),
        .d_kind(d_kind), .d_qp_y(d_qp_y), .d_qp_cb(d_qp_cb), .d_qp_cr(d_qp_cr),
        .d_mb_x(d_mb_x), .d_i16_mode(d_i16_mode), .d_c_mode(d_c_mode),
        .d_i4_modes(d_i4_modes), .d_avail(d_avail),
        .d_mb_y(d_mb_y), .d_last_col(d_last_col), .d_last_row(d_last_row),
        .d_frame(d_frame), .d_filt(d_filt), .d_t8x8(d_t8x8), .d_bs(d_bs),
        .d_qps(d_qps), .d_off_a(d_off_a), .d_off_b(d_off_b)
    );

    // The descriptor ring, one copy a reader.
    reg [20:0]  ring1 [0:7];    // B1: kind, QPs
    reg [81:0]  ring2 [0:7];    // B2: position, modes, availability
    reg [183:0] ring3 [0:7];    // C: everything the filter wants

    always @(posedge clk) begin
        if (d_we) begin
            ring1[d_idx] <= {d_kind, d_qp_y, d_qp_cb, d_qp_cr};
            ring2[d_idx] <= {d_mb_x, d_kind, d_i16_mode, d_c_mode, d_i4_modes, d_avail};
            ring3[d_idx] <= {d_mb_x, d_mb_y, d_last_col, d_last_row, d_frame, d_filt, d_t8x8,
                             d_bs, d_qps, d_off_a, d_off_b};
        end
    end

    wire [20:0]  r1 = ring1[b1_cnt];
    wire [81:0]  r2 = ring2[b2_cnt];
    wire [183:0] r3 = ring3[c_cnt];

    // -----------------------------------------------------------------
    // Stage B1
    // -----------------------------------------------------------------
    reg         b1_run;
    wire        b1_busy, b1_done;
    wire [7:0]  rr_addr;
    wire [39:0] rr_data;
    wire [23:0] zf0, zf1;
    wire [2:0]  b1_ahead_b2 = b1_cnt - b2_cnt;
    wire        b1_go = !b1_run && (a_cnt != b1_cnt) && (b1_ahead_b2 <= 3'd1);

    h264_residual residual (
        .clk(clk), .rst_n(rst_n),
        .cw_en(cw_en), .cw_addr(cw_addr), .cw_data(cw_data),
        .mw_en(mw_en), .mw_addr(mw_addr), .mw_mask(mw_mask),
        .clr_en(clr_en), .clr_half(clr_half),
        .go(b1_go), .half(b1_cnt[0]), .kind(r1[20:18]),
        .qp_y(r1[17:12]), .qp_cb(r1[11:6]), .qp_cr(r1[5:0]),
        .busy(b1_busy), .done(b1_done),
        .rr_addr(rr_addr), .rr_data(rr_data), .zf0(zf0), .zf1(zf1)
    );

    // -----------------------------------------------------------------
    // Stage B2
    // -----------------------------------------------------------------
    reg         b2_run;
    wire        b2_busy, b2_done;
    wire        ow_en;
    wire [6:0]  ow_addr;
    wire [31:0] ow_data;
    wire [2:0]  b2_ahead_c = b2_cnt - c_cnt;
    wire        b2_go = !b2_run && (b1_cnt != b2_cnt) && (b2_ahead_c <= 3'd2);

    h264_intra #(.MAX_W_MBS(MAX_W_MBS)) intra (
        .clk(clk), .rst_n(rst_n),
        .go(b2_go), .busy(b2_busy), .done(b2_done),
        .mb_x(r2[81:75]), .kind(r2[74:72]), .i16_mode(r2[71:70]), .c_mode(r2[69:68]),
        .i4_modes(r2[67:4]),
        .avail_a(r2[0]), .avail_b(r2[1]), .avail_c(r2[2]), .avail_d(r2[3]),
        .half(b2_cnt[0]),
        .rr_addr(rr_addr), .rr_data(rr_data), .zf(b2_cnt[0] ? zf1 : zf0),
        .pred_valid(pred_valid), .pred_ready(pred_ready), .pred_data(pred_data),
        .ow_en(ow_en), .ow_addr(ow_addr), .ow_data(ow_data)
    );

    // -----------------------------------------------------------------
    // Stage C
    // -----------------------------------------------------------------
    reg         c_run;
    wire        c_busy, c_done;
    wire        c_go = !c_run && (b2_cnt != c_cnt);

    wire [6:0]  cur_ra, cur_wa, left_ra, left_wa;
    wire        cur_we, left_we;
    wire [31:0] cur_wd, left_wd;
    reg  [31:0] cur_rd, left_rd;
    wire        op_valid, op_room;
    wire [33:0] op_data;

    h264_deblock #(.MAX_W_MBS(MAX_W_MBS)) deblock (
        .clk(clk), .rst_n(rst_n),
        .go(c_go), .busy(c_busy), .done(c_done),
        .mb_x(r3[183:177]), .mb_y(r3[176:170]),
        .last_col(r3[169]), .last_row(r3[168]), .frame(r3[167:164]),
        .filt_left(r3[161]), .filt_top(r3[162]), .filt_inner(r3[163]),
        .t8x8(r3[160]), .bs(r3[159:64]),
        .qp_y(r3[15:10]), .qp_y_left(r3[21:16]), .qp_y_top(r3[27:22]),
        .qp_cb(r3[33:28]), .qp_cb_left(r3[39:34]), .qp_cb_top(r3[45:40]),
        .qp_cr(r3[51:46]), .qp_cr_left(r3[57:52]), .qp_cr_top(r3[63:58]),
        .off_a(r3[9:5]), .off_b(r3[4:0]),
        .cur_ra(cur_ra), .cur_rd(cur_rd), .cur_we(cur_we), .cur_wa(cur_wa), .cur_wd(cur_wd),
        .left_ra(left_ra), .left_rd(left_rd), .left_we(left_we), .left_wa(left_wa),
        .left_wd(left_wd),
        .op_valid(op_valid), .op_data(op_data), .op_room(op_room)
    );

    // -----------------------------------------------------------------
    // The four macroblock slots
    // -----------------------------------------------------------------
    wire [1:0] s_b2   = b2_cnt[1:0];
    wire [1:0] s_cur  = c_cnt[1:0];
    wire [1:0] s_left = c_cnt[1:0] - 2'd1;

    reg [31:0] slot0 [0:127];
    reg [31:0] slot1 [0:127];
    reg [31:0] slot2 [0:127];
    reg [31:0] slot3 [0:127];
    reg [31:0] sq0, sq1, sq2, sq3;

    // Each slot's one write: B2's when it is B2's, else the deblocker's
    // through whichever role the slot has. B2's slot is never the
    // deblocker's current or left one (it is one or two ahead).
    function [39:0] slot_w;            // {we, addr[6:0], data}
        input [1:0] s;
        begin
            if (s == s_b2 && ow_en)          slot_w = {1'b1, ow_addr, ow_data};
            else if (s == s_cur && cur_we)   slot_w = {1'b1, cur_wa, cur_wd};
            else if (s == s_left && left_we) slot_w = {1'b1, left_wa, left_wd};
            else                             slot_w = 40'd0;
        end
    endfunction

    wire [39:0] w0 = slot_w(2'd0);
    wire [39:0] w1 = slot_w(2'd1);
    wire [39:0] w2 = slot_w(2'd2);
    wire [39:0] w3 = slot_w(2'd3);
    wire [6:0]  ra0 = (s_cur == 2'd0) ? cur_ra : left_ra;
    wire [6:0]  ra1 = (s_cur == 2'd1) ? cur_ra : left_ra;
    wire [6:0]  ra2 = (s_cur == 2'd2) ? cur_ra : left_ra;
    wire [6:0]  ra3 = (s_cur == 2'd3) ? cur_ra : left_ra;

    integer n;
    initial begin
        for (n = 0; n < 128; n = n + 1) begin
            slot0[n] = 32'd0; slot1[n] = 32'd0; slot2[n] = 32'd0; slot3[n] = 32'd0;
        end
    end

    always @(posedge clk) begin
        if (w0[39]) slot0[w0[38:32]] <= w0[31:0];
        if (w1[39]) slot1[w1[38:32]] <= w1[31:0];
        if (w2[39]) slot2[w2[38:32]] <= w2[31:0];
        if (w3[39]) slot3[w3[38:32]] <= w3[31:0];
        sq0 <= slot0[ra0];
        sq1 <= slot1[ra1];
        sq2 <= slot2[ra2];
        sq3 <= slot3[ra3];
    end

    // Which slot answered last cycle's read: the roles move only when
    // the deblocker finishes, so the role it read with is the role now.
    always @* begin
        case (s_cur)
            2'd0:    cur_rd = sq0;
            2'd1:    cur_rd = sq1;
            2'd2:    cur_rd = sq2;
            default: cur_rd = sq3;
        endcase
        case (s_left)
            2'd0:    left_rd = sq0;
            2'd1:    left_rd = sq1;
            2'd2:    left_rd = sq2;
            default: left_rd = sq3;
        endcase
    end

    // -----------------------------------------------------------------
    // Stage D
    // -----------------------------------------------------------------
    h264_frame_wr writer (
        .clk(clk), .rst_n(rst_n),
        .op_valid(op_valid), .op_data(op_data), .op_room(op_room),
        .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_we(cmd_we),
        .cmd_addr(cmd_addr), .cmd_len(cmd_len),
        .wr_valid(wr_valid), .wr_ready(wr_ready), .wr_data(wr_data), .wr_be(wr_be),
        .pic_done(pic_done)
    );

    // -----------------------------------------------------------------
    // The counters
    // -----------------------------------------------------------------
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            b1_cnt <= 3'd0; b2_cnt <= 3'd0; c_cnt <= 3'd0;
            b1_run <= 1'b0; b2_run <= 1'b0; c_run <= 1'b0;
        end else begin
            if (b1_go) b1_run <= 1'b1;
            if (b1_done) begin
                b1_run <= 1'b0;
                b1_cnt <= b1_cnt + 3'd1;
            end
            if (b2_go) b2_run <= 1'b1;
            if (b2_done) begin
                b2_run <= 1'b0;
                b2_cnt <= b2_cnt + 3'd1;
            end
            if (c_go) c_run <= 1'b1;
            if (c_done) begin
                c_run <= 1'b0;
                c_cnt <= c_cnt + 3'd1;
            end
        end
    end

    // The read half of the port is not used: reference pictures are
    // h264_inter's to fetch.
    wire unused_rd = &{1'b0, rd_valid, rd_data, rd_last, b1_busy, b2_busy, c_busy};
endmodule
