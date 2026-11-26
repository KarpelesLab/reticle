// h264_cavlc -- one residual_block_cavlc() of ITU-T H.264 7.3.5.3.2,
// decoded into `COEF_BLK` and `COEF` tokens.
//
// The parser starts it with the block's coeff_token class (the nC it
// derived), its maxNumCoeff and its token category and index, and hands it
// the bit window until `done`. Each symbol takes two phases: on `latch` the
// window is decoded into registers, and on a later `go` (the window full and
// the output slot free) the state acts on them, consumes and emits. A step
// that reads nothing (`reads` low) takes `go` alone, and a level needs one
// cycle more between the two (`hold`), because the suffix depends on the
// prefix and levelCode on both.
//
// Cycles, for a block of TotalCoeff n with t trailing ones and r runs
// actually coded (a run_before is absent once zerosLeft reaches 0):
//
//   2                coeff_token and the t sign bits, and the COEF_BLK token
//   3 (n - t)        one level each: level_prefix, level_suffix, suffixLength
//   2, or 1          total_zeros, or its absence (TotalCoeff = maxNumCoeff)
//   n + r            one COEF token each; one with a run_before takes two
//
// so about 5n for a dense block, and 2 for a block with no coefficients.
// Every symbol is decoded whole from the window; nothing here is bit-serial.
// The cycles went from one a symbol to two or three to meet a 10 ns clock:
// README.md *Timing* has the measurement and why the trade was cheap.
//
// The order of the COEF tokens is the order CAVLC codes them, highest scan
// position first (9.2.4 places levelVal[0] last in the scan).
// docs/h264-decoder.md §1.6 is the format.
//
// Limits, and what is reported rather than decoded
//   level_prefix above 16 is reported (`err`): the window is 32 bits and a
//   prefix of 17 wants 17 + 1 + 14. The Baseline, Main and Extended
//   profiles cap level_prefix at 15 (9.2.2.1's note), so only a High
//   profile stream at a very low QP can reach it. Also reported: no
//   coeff_token matching, TotalCoeff above maxNumCoeff, no total_zeros
//   matching or TotalCoeff + total_zeros above maxNumCoeff, and a run_before
//   longer than the zeros left -- each a stream 9.2 does not allow.
module h264_cavlc (
    input  wire        clk,
    input  wire        rst_n,

    input  wire        start,
    // Back to idle, whatever it was doing: the parser gave up on the NAL
    // unit in the middle of a block.
    input  wire        abort,
    // Table 9-5 column: 0..3 for nC in [0,2) [2,4) [4,8) [8,..), 4 for -1.
    input  wire [2:0]  cls,
    // maxNumCoeff: 2'd0 for 16 (a 4x4 block from position 0), 2'd1 for 15
    // (an AC block, positions 1..15), 2'd2 for 4 (chroma DC).
    input  wire [1:0]  kind,
    input  wire [3:0]  cat,
    input  wire [3:0]  blk,

    input  wire [31:0] win,
    // Two phases a symbol: `latch` decodes the window into registers, and
    // `go`, on a later cycle, acts on them, consumes and emits. A state
    // that reads no bits (`reads` low) takes `go` alone.
    input  wire        latch,
    input  wire        go,
    output wire        reads,
    // The second step of a level's decode is still running: no `go` yet.
    output wire        hold,
    // The latched length of the symbol being acted on: what the second
    // phase consumes, as a register.
    output wire [5:0]  dlen,
    output reg  [5:0]  consume,
    output reg         tok_we,
    output reg  [31:0] tok,

    output wire        busy,
    output reg         done,
    output reg         err,
    output reg  [4:0]  tc_out
);
    localparam [2:0] C_IDLE = 3'd0, C_TOK = 3'd1, C_LVL = 3'd2, C_TZ = 3'd3, C_RUN = 3'd4;

    reg [2:0]  st_q;
    reg [2:0]  cls_q;
    reg [1:0]  kind_q;
    reg [3:0]  cat_q;
    reg [3:0]  blk_q;
    reg [4:0]  tc_q;
    reg [1:0]  t1_q;
    reg [4:0]  i_q;
    reg [2:0]  sl_q;
    reg [4:0]  zl_q;
    reg [5:0]  pos_q;
    reg [255:0] lv_v;

    // The decode of the window, latched in the first phase.
    reg        d_ctv;
    reg [4:0]  d_tc;
    reg [1:0]  d_t1;
    reg [2:0]  d_signs;
    reg        d_bad;
    reg [2:0]  d_sl;
    reg        d_tzv;
    reg [3:0]  d_tz;
    reg        d_rbv;
    reg [3:0]  d_rb;
    reg [5:0]  d_len;

    function [15:0] pick16x16;
        input [255:0] v;
        input [3:0] i;
        begin
            case (i)
            4'd0: pick16x16 = v[15:0];
            4'd1: pick16x16 = v[31:16];
            4'd2: pick16x16 = v[47:32];
            4'd3: pick16x16 = v[63:48];
            4'd4: pick16x16 = v[79:64];
            4'd5: pick16x16 = v[95:80];
            4'd6: pick16x16 = v[111:96];
            4'd7: pick16x16 = v[127:112];
            4'd8: pick16x16 = v[143:128];
            4'd9: pick16x16 = v[159:144];
            4'd10: pick16x16 = v[175:160];
            4'd11: pick16x16 = v[191:176];
            4'd12: pick16x16 = v[207:192];
            4'd13: pick16x16 = v[223:208];
            4'd14: pick16x16 = v[239:224];
            4'd15: pick16x16 = v[255:240];
            default: pick16x16 = 16'd0;
            endcase
        end
    endfunction
    function [255:0] put16x16;
        input [255:0] v;
        input [3:0] i;
        input [15:0] x;
        begin
            put16x16 = v;
            case (i)
            4'd0: put16x16[15:0] = x;
            4'd1: put16x16[31:16] = x;
            4'd2: put16x16[47:32] = x;
            4'd3: put16x16[63:48] = x;
            4'd4: put16x16[79:64] = x;
            4'd5: put16x16[95:80] = x;
            4'd6: put16x16[111:96] = x;
            4'd7: put16x16[127:112] = x;
            4'd8: put16x16[143:128] = x;
            4'd9: put16x16[159:144] = x;
            4'd10: put16x16[175:160] = x;
            4'd11: put16x16[191:176] = x;
            4'd12: put16x16[207:192] = x;
            4'd13: put16x16[223:208] = x;
            4'd14: put16x16[239:224] = x;
            4'd15: put16x16[255:240] = x;
            default: ;
            endcase
        end
    endfunction

    wire [15:0] lv_cur = pick16x16(lv_v, i_q[3:0]);
    assign busy = (st_q != C_IDLE);

    // maxNumCoeff and the first scan position of this kind of block.
    wire [4:0] maxn  = (kind_q == 2'd0) ? 5'd16 : (kind_q == 2'd1) ? 5'd15 : 5'd4;
    wire [5:0] first = (kind_q == 2'd1) ? 6'd1 : 6'd0;

    // ---- the tables ------------------------------------------------------
    wire       ct_valid;
    wire [4:0] ct_len;
    wire [4:0] ct_len1;
    wire [1:0] ct_t1;
    wire [4:0] ct_tc;
    wire       tz_valid;
    wire [3:0] tz_len;
    wire [3:0] tz_val;
    wire       rb_valid;
    wire [3:0] rb_len;
    wire [3:0] rb_val;
    wire       cbp_valid_unused;
    wire [5:0] cbp_val_unused;
    wire [2:0] zl_sel = (zl_q > 5'd6) ? 3'd7 : zl_q[2:0];

    h264_vlc vlc (
        .ct_cls(cls_q), .ct_bits(win[31:16]),
        .ct_valid(ct_valid), .ct_len(ct_len), .ct_len1(ct_len1), .ct_t1(ct_t1), .ct_tc(ct_tc),
        .tz_dc(kind_q == 2'd2), .tz_tc(tc_q[3:0]), .tz_bits(win[31:23]),
        .tz_valid(tz_valid), .tz_len(tz_len), .tz_val(tz_val),
        .rb_zl(zl_sel), .rb_bits(win[31:21]),
        .rb_valid(rb_valid), .rb_len(rb_len), .rb_val(rb_val),
        .cbp_code(6'd0), .cbp_inter(1'b0),
        .cbp_valid(cbp_valid_unused), .cbp_val(cbp_val_unused)
    );

    // ---- coeff_token and the trailing ones' signs ---------------------------
    wire [31:0] after_ct = win << ct_len;
    wire [2:0]  signs    = after_ct[31:29];

    // ---- one level -------------------------------------------------------
    // level_prefix: the leading zeros of the window.
    // Leading zeros of a 32-bit word as a tree, 32 for none: halving the
    // word five times. A loop that keeps the last set bit maps to a chain
    // of 32 multiplexers instead, which was this block's longest path.
    function [5:0] clz32;
        input [31:0] w;
        reg   [15:0] a;
        reg   [7:0]  b;
        reg   [3:0]  c;
        reg   [1:0]  e;
        reg          z4, z3, z2, z1;
        begin
            z4 = ~|w[31:16];
            a  = z4 ? w[15:0] : w[31:16];
            z3 = ~|a[15:8];
            b  = z3 ? a[7:0] : a[15:8];
            z2 = ~|b[7:4];
            c  = z2 ? b[3:0] : b[7:4];
            z1 = ~|c[3:2];
            e  = z1 ? c[1:0] : c[3:2];
            clz32 = (w == 32'd0) ? 6'd32 : {1'b0, z4, z3, z2, z1, ~e[1]};
        end
    endfunction
    wire [5:0] lz = clz32(win);
    wire [31:0] after_pre = win << (lz[4:0] + 5'd1);
    // A level is decoded in two latched steps: the prefix and the 13 bits
    // after it first, then the suffix and levelCode from those.
    reg        lvl_b;
    reg [5:0]  d_pre;
    reg [12:0] d_after;
    wire [4:0] prefix = d_pre[4:0];
    wire       prefix_bad = (d_pre > 6'd16);
    wire [3:0] ssize = (prefix == 5'd14 && sl_q == 3'd0) ? 4'd4 :
                       (prefix >= 5'd15) ? (prefix[3:0] - 4'd3) :
                       {1'b0, sl_q};
    wire [12:0] suffix = (ssize == 4'd0) ? 13'd0 : (d_after >> (4'd13 - ssize));
    wire [3:0]  pmin = (prefix >= 5'd15) ? 4'd15 : prefix[3:0];
    wire [14:0] lcode = ({11'd0, pmin} << sl_q)
                      + {2'd0, suffix}
                      + ((prefix >= 5'd15 && sl_q == 3'd0) ? 15'd15 : 15'd0)
                      + ((prefix >= 5'd16) ? 15'd4096 : 15'd0)
                      + ((i_q == {3'd0, t1_q} && t1_q != 2'd3) ? 15'd2 : 15'd0);
    // levelVal: (levelCode + 2) >> 1 for an even code, (-levelCode - 1) >> 1
    // for an odd one; the magnitude is (levelCode >> 1) + 1 either way.
    // |levelVal| = (levelCode >> 1) + 1, so |levelVal| > 3 << (sl - 1) is
    // levelCode >= 6 << (sl - 1): compared on levelCode, with no adder.
    wire [2:0]  sl1   = (sl_q == 3'd0) ? 3'd1 : sl_q;
    wire [14:0] thr2  = 15'd6 << (sl1 - 3'd1);
    wire [2:0]  sl2   = (lcode >= thr2 && sl1 < 3'd6) ? sl1 + 3'd1 : sl1;
    // The level from the latched levelCode: (c + 2) >> 1 for an even code
    // and (-c - 1) >> 1, which is ~(c >> 1), for an odd one.
    reg  [14:0] d_lcode;
    wire [15:0] level = d_lcode[0] ? ~{2'b00, d_lcode[14:1]} : ({2'b00, d_lcode[14:1]} + 16'd1);
    wire [5:0]  lvl_len = {1'b0, prefix} + 6'd1 + {2'd0, ssize};

    // ---- total_zeros -------------------------------------------------------
    wire       need_tz = (tc_q < maxn);
    wire [4:0] tz_sum  = tc_q + {1'b0, tz_val};

    // ---- run_before ----------------------------------------------------------
    wire       last_coef = (i_q == tc_q - 5'd1);

    assign hold = lvl_b;
    assign dlen = d_len;
    assign reads = (st_q == C_TOK) || (st_q == C_LVL) || (st_q == C_TZ && need_tz) ||
                   (st_q == C_RUN && !last_coef && zl_q != 5'd0);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            d_ctv <= 1'b0; d_tc <= 5'd0; d_t1 <= 2'd0; d_signs <= 3'd0;
            d_bad <= 1'b0; d_lcode <= 15'd0; d_sl <= 3'd0;
            lvl_b <= 1'b0; d_pre <= 6'd0; d_after <= 13'd0;
            d_tzv <= 1'b0; d_tz <= 4'd0; d_rbv <= 1'b0; d_rb <= 4'd0;
            d_len <= 6'd0;
        end else if (abort) begin
            lvl_b <= 1'b0;
        end else if (lvl_b) begin
            d_bad   <= prefix_bad;
            d_lcode <= lcode;
            d_sl    <= sl2;
            d_len   <= lvl_len;
            lvl_b   <= 1'b0;
        end else if (latch) begin
            d_ctv   <= ct_valid && ct_tc <= maxn;
            d_tc    <= ct_tc;
            d_t1    <= ct_t1;
            d_signs <= signs;
            d_pre   <= lz;
            d_after <= after_pre[31:19];
            lvl_b   <= (st_q == C_LVL);
            d_tzv   <= tz_valid;
            d_tz    <= tz_val;
            d_rbv   <= rb_valid && {1'b0, rb_val} <= zl_q;
            d_rb    <= rb_val;
            case (st_q)
                C_TOK:   d_len <= {1'b0, ct_len1};
                C_LVL:   d_len <= 6'd0;
                C_TZ:    d_len <= {2'd0, tz_len};
                default: d_len <= {2'd0, rb_len};
            endcase
        end
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            st_q   <= C_IDLE;
            cls_q  <= 3'd0;
            kind_q <= 2'd0;
            cat_q  <= 4'd0;
            blk_q  <= 4'd0;
            tc_q   <= 5'd0;
            t1_q   <= 2'd0;
            i_q    <= 5'd0;
            sl_q   <= 3'd0;
            zl_q   <= 5'd0;
            pos_q  <= 6'd0;
            lv_v   <= 256'd0;
        end else if (abort) begin
            st_q <= C_IDLE;
        end else begin
            case (st_q)
            C_IDLE:
                if (start) begin
                    cls_q  <= cls;
                    kind_q <= kind;
                    cat_q  <= cat;
                    blk_q  <= blk;
                    st_q   <= C_TOK;
                end
            C_TOK:
                if (go) begin
                    if (!d_ctv) begin
                        st_q <= C_IDLE;
                    end else begin
                        tc_q <= d_tc;
                        t1_q <= d_t1;
                        i_q  <= {3'd0, d_t1};
                        sl_q <= (d_tc > 5'd10 && d_t1 != 2'd3) ? 3'd1 : 3'd0;
                        lv_v[0 +: 16] <= d_signs[2] ? 16'hFFFF : 16'd1;
                        lv_v[16 +: 16] <= d_signs[1] ? 16'hFFFF : 16'd1;
                        lv_v[32 +: 16] <= d_signs[0] ? 16'hFFFF : 16'd1;
                        if (d_tc == 5'd0)
                            st_q <= C_IDLE;
                        else if ({3'd0, d_t1} == d_tc)
                            st_q <= C_TZ;
                        else
                            st_q <= C_LVL;
                    end
                end
            C_LVL:
                if (go) begin
                    if (d_bad) begin
                        st_q <= C_IDLE;
                    end else begin
                        lv_v <= put16x16(lv_v, i_q[3:0], level);
                        sl_q <= d_sl;
                        i_q  <= i_q + 5'd1;
                        if (i_q + 5'd1 == tc_q)
                            st_q <= C_TZ;
                    end
                end
            C_TZ:
                if (go) begin
                    if (need_tz && (!d_tzv || tc_q + {1'b0, d_tz} > maxn)) begin
                        st_q <= C_IDLE;
                    end else begin
                        zl_q  <= need_tz ? {1'b0, d_tz} : 5'd0;
                        pos_q <= first + {1'b0, tc_q} - 6'd1 + (need_tz ? {2'd0, d_tz} : 6'd0);
                        i_q   <= 5'd0;
                        st_q  <= C_RUN;
                    end
                end
            C_RUN:
                if (go) begin
                    if (last_coef) begin
                        st_q <= C_IDLE;
                    end else if (zl_q != 5'd0 && !d_rbv) begin
                        st_q <= C_IDLE;
                    end else begin
                        i_q <= i_q + 5'd1;
                        if (zl_q != 5'd0) begin
                            zl_q  <= zl_q - {1'b0, d_rb};
                            pos_q <= pos_q - {2'd0, d_rb} - 6'd1;
                        end else begin
                            pos_q <= pos_q - 6'd1;
                        end
                    end
                end
            default:
                st_q <= C_IDLE;
            endcase
        end
    end

    // What this cycle consumes, emits and reports. Only meaningful with
    // `go`; the parser gates it.
    always @* begin
        consume = 6'd0;
        tok_we  = 1'b0;
        tok     = 32'd0;
        done    = 1'b0;
        err     = 1'b0;
        tc_out  = tc_q;
        case (st_q)
        C_TOK: begin
            tok_we = 1'b1;
            tok    = {8'h20, cat_q, blk_q, 11'd0, d_tc};
            consume = d_len;
            tc_out = d_tc;
            if (!d_ctv) begin
                err    = 1'b1;
                tok_we = 1'b0;
            end else if (d_tc == 5'd0) begin
                done = 1'b1;
            end
        end
        C_LVL: begin
            consume = d_len;
            err = d_bad;
        end
        C_TZ: begin
            consume = need_tz ? d_len : 6'd0;
            err = need_tz && (!d_tzv || tc_q + {1'b0, d_tz} > maxn);
        end
        C_RUN: begin
            tok_we = 1'b1;
            tok    = {8'h21, pos_q, 2'd0, lv_cur};
            if (last_coef) begin
                done = 1'b1;
            end else if (zl_q != 5'd0) begin
                consume = d_len;
                if (!d_rbv) begin
                    err    = 1'b1;
                    tok_we = 1'b0;
                end
            end
        end
        default: ;
        endcase
    end
endmodule
