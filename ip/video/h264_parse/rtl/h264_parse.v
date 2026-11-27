// h264_parse -- the front end of an H.264 decoder: an Annex B byte
// stream in, the syntax token stream of docs/h264-decoder.md §1 out.
//
// What it does
//   Everything between the bytes and the residual: start codes and
//   emulation prevention (`h264_nal`), a 32-bit bit window (`h264_bits`),
//   sequence and picture parameter sets stored by id, the slice header
//   with its reference list modification, weight table and reference
//   marking surfaced as tokens for a DPB manager outside, and the slice
//   data of CAVLC I and P slices -- mb_skip_run, macroblock_layer,
//   mb_pred, sub_mb_pred, coded_block_pattern, mb_qp_delta, I_PCM, and
//   every residual block through `h264_cavlc`. Section numbers below are
//   ITU-T H.264 (V16, 06/2026)'s.
//
//   The parser resolves what needs the neighbours: the Intra4x4PredMode
//   and Intra8x8PredMode of 8.3.1.1 / 8.3.2.1 and the nC of 9.2.1 for
//   every coeff_token. Both need the macroblock above, so a line buffer
//   of one 57-bit word per macroblock column, in block RAM, holds the
//   bottom row of each macroblock's TotalCoeffs (luma and both chromas),
//   its bottom row of intra modes and whether it was inter. The
//   macroblock to the left is in registers.
//
// What it reports instead
//   An `ERROR` token (docs/h264-decoder.md §1.8) for anything outside
//   Baseline-with-the-8x8-transform as this version decodes it: CABAC, B,
//   SP and SI slices, FMO, interlace, chroma other than 4:2:0, bit depth
//   other than 8, scaling matrices, pictures wider than `MAX_W_MBS`. And
//   for a stream that breaks the syntax. Then it drops the NAL unit and
//   carries on at the next one.
//
// One state machine
//   Like `ip/compress/inflate`, this is one sequential machine over one
//   datapath. Each state reads at most one syntax element -- every
//   element is decoded whole from the window, never bit by bit -- and
//   emits at most one token, into `tok_q`, which is the output register.
//   A state that reads takes two phases: the first latches the decode of
//   the window (`latch`), the second acts on it, consumes and emits (`go`,
//   when the output slot is free). A state that reads nothing takes `go`
//   alone. `consume`, the one output to the bit reader that matters for
//   timing, is a function of registers and of the output slot; README.md
//   *Timing* is why.
//
// Exp-Golomb
//   ue(v) is decoded in one phase when its code fits the window, which is
//   every code of 31 bits or fewer: values up to 65534. A longer code
//   (32..63 bits, values to 2^32 - 2) takes three more states through
//   `S_LONG1` to `S_LONG3` and returns to the state that asked, which then
//   finds the value in `long_val_q`. No syntax element this parser keeps
//   is that large in a stream it accepts; the path exists so that the
//   ones it skips (offset_for_ref_frame, idr_pic_id) are skipped
//   correctly whatever they hold.
module h264_parse #(
    // The widest picture, in macroblocks: the line buffer's depth. 128
    // is 2048 luma samples, which covers 720p (80) and 1080p (120).
    parameter MAX_W_MBS = 128
) (
    input  wire        clk,
    input  wire        rst_n,

    // The Annex B byte stream. `in_last` with the final byte ends the
    // stream, so that its last NAL unit is not held waiting for a start
    // code that will never come (h264_nal's header).
    input  wire [7:0]  in_byte,
    input  wire        in_valid,
    input  wire        in_last,
    output wire        in_ready,

    output wire [31:0] tok,
    output wire        tok_valid,
    input  wire        tok_ready
);
    // ======================================================================
    // The byte stream and the bit window
    // ======================================================================
    wire [7:0] nb_byte;
    wire       nb_first, nb_last, nb_valid, nb_ready;

    h264_nal u_nal (
        .clk(clk), .rst_n(rst_n),
        .in_byte(in_byte), .in_valid(in_valid), .in_last(in_last), .in_ready(in_ready),
        .out_byte(nb_byte), .out_first(nb_first), .out_last(nb_last),
        .out_valid(nb_valid), .out_ready(nb_ready)
    );

    wire [31:0] win;
    wire        win_ok, at_end, more, more_ok, aligned;
    wire [2:0]  bitpos;
    wire [6:0]  avail;
    reg  [5:0]  consume;
    wire        short;
    wire        drop;

    h264_bits u_bits (
        .clk(clk), .rst_n(rst_n),
        .in_byte(nb_byte), .in_first(nb_first), .in_last(nb_last),
        .in_valid(nb_valid), .in_ready(nb_ready),
        .win(win), .win_ok(win_ok), .avail(avail), .at_end(at_end),
        .more(more), .more_ok(more_ok), .aligned(aligned), .bitpos(bitpos),
        .consume(consume), .drop(drop), .short(short)
    );

    // ======================================================================
    // States
    // ======================================================================
    localparam [7:0]
        S_NAL       = 8'd0,   S_DROP      = 8'd1,   S_ERR       = 8'd2,
        S_LONG1     = 8'd3,   S_LONG2     = 8'd4,   S_LONG3     = 8'd5,
        // sequence parameter set
        S_SPS_PROF  = 8'd10,  S_SPS_ID    = 8'd11,  S_SPS_CF    = 8'd12,
        S_SPS_SEP   = 8'd13,  S_SPS_BDY   = 8'd14,  S_SPS_BDC   = 8'd15,
        S_SPS_QPP   = 8'd16,  S_SPS_SCF   = 8'd17,  S_SPS_LMFN  = 8'd18,
        S_SPS_POCT  = 8'd19,  S_SPS_LMPOC = 8'd20,  S_SPS_DPOAZ = 8'd21,
        S_SPS_ONRP  = 8'd22,  S_SPS_OTTB  = 8'd23,  S_SPS_NCYC  = 8'd24,
        S_SPS_OFR   = 8'd25,  S_SPS_MAXR  = 8'd26,  S_SPS_GAPS  = 8'd27,
        S_SPS_W     = 8'd28,  S_SPS_H     = 8'd29,  S_SPS_FMO   = 8'd30,
        S_SPS_MBAFF = 8'd31,  S_SPS_D8    = 8'd32,  S_SPS_CROPF = 8'd33,
        S_SPS_CROP  = 8'd34,  S_SPS_STORE = 8'd35,
        // scaling lists, shared by both parameter sets
        S_SCL_FLAG  = 8'd36,  S_SCL_DELTA = 8'd37,
        // picture parameter set
        S_PPS_ID    = 8'd40,  S_PPS_SPS   = 8'd41,  S_PPS_ENT   = 8'd42,
        S_PPS_BFP   = 8'd43,  S_PPS_NSG   = 8'd44,  S_PPS_L0    = 8'd45,
        S_PPS_L1    = 8'd46,  S_PPS_WP    = 8'd47,  S_PPS_WB    = 8'd48,
        S_PPS_QP    = 8'd49,  S_PPS_QS    = 8'd50,  S_PPS_CQP   = 8'd51,
        S_PPS_DFC   = 8'd52,  S_PPS_CIP   = 8'd53,  S_PPS_RPC   = 8'd54,
        S_PPS_MORE  = 8'd55,  S_PPS_T8    = 8'd56,  S_PPS_SCF   = 8'd57,
        S_PPS_CQP2  = 8'd58,  S_PPS_STORE = 8'd59,
        // slice header
        S_SH_FIRST  = 8'd60,  S_SH_TYPE   = 8'd61,  S_SH_PPS    = 8'd62,
        S_SH_ACT1   = 8'd63,  S_SH_ACT2   = 8'd64,  S_SH_ACT3   = 8'd65,
        S_SH_ACT4   = 8'd66,  S_SH_FN     = 8'd67,  S_SH_IDR    = 8'd68,
        S_SH_POCL   = 8'd69,  S_SH_DPB    = 8'd70,  S_SH_DP0    = 8'd71,
        S_SH_DP1    = 8'd72,  S_SH_RPC    = 8'd73,
        S_E_DIMS    = 8'd74,  S_E_INFO    = 8'd75,  S_E_POCT    = 8'd76,
        S_E_CROP    = 8'd77,  S_E_HDR     = 8'd78,  S_E_FIRST   = 8'd79,
        S_E_POC0    = 8'd80,  S_E_POC1    = 8'd81,
        S_SH_OVR    = 8'd82,  S_SH_L0     = 8'd83,  S_E_REF     = 8'd84,
        S_RPLM_F    = 8'd85,  S_RPLM_IDC  = 8'd86,  S_RPLM_V    = 8'd87,
        S_PWT_LD    = 8'd88,  S_PWT_CD    = 8'd89,  S_PWT_LF    = 8'd90,
        S_PWT_LW    = 8'd91,  S_PWT_LO    = 8'd92,  S_PWT_CF    = 8'd93,
        S_PWT_CW    = 8'd94,  S_PWT_CO    = 8'd95,
        S_DRPM_IDR  = 8'd96,  S_DRPM_A    = 8'd97,  S_MMCO_OP   = 8'd98,
        S_MMCO_X    = 8'd99,  S_MMCO_Y    = 8'd100, S_SH_QPD    = 8'd101,
        S_SH_DFI    = 8'd102, S_SH_FA     = 8'd103, S_SH_FB     = 8'd104,
        S_E_QP      = 8'd105, S_E_DBLK    = 8'd106, S_SD_DIV    = 8'd107,
        // slice data
        S_SD_LOOP   = 8'd110, S_SKIPRUN   = 8'd111, S_SK_START  = 8'd112,
        S_SK_TYPE   = 8'd113, S_SK_CBP    = 8'd114, S_SK_QP     = 8'd115,
        S_MB_END    = 8'd116, S_AFTER     = 8'd117, S_SLICE_END = 8'd118,
        S_PIC_END   = 8'd119,
        S_MB_START  = 8'd120, S_MB_TYPE   = 8'd121, S_PCM_CBP   = 8'd122,
        S_PCM_QP    = 8'd123, S_PCM_ALIGN = 8'd124, S_PCM_DATA  = 8'd125,
        S_SUB       = 8'd126, S_SUB_TOK   = 8'd127, S_T8A       = 8'd128,
        S_IPRED     = 8'd129, S_CPM       = 8'd130, S_REF       = 8'd131,
        S_MVD       = 8'd132, S_CBP       = 8'd133, S_T8B       = 8'd134,
        S_CBPTOK    = 8'd135, S_QPD       = 8'd136, S_QPTOK     = 8'd137,
        S_RES       = 8'd138, S_RESW      = 8'd139, S_RES2      = 8'd140;

    // MB_TYPE kinds, docs/h264-decoder.md §1.4.
    localparam [4:0] K_INXN = 5'd0, K_I16 = 5'd1, K_PCM = 5'd2,
                     K_P16  = 5'd4, K_P168 = 5'd5, K_P816 = 5'd6,
                     K_P8   = 5'd7, K_P8R0 = 5'd8, K_PSKIP = 5'd9;

    // Error codes, docs/h264-decoder.md §1.8.
    localparam [7:0] E_SPS = 8'd1, E_PPS = 8'd2, E_MISSING = 8'd3,
                     E_UNSUP = 8'd4, E_SYNTAX = 8'd5, E_SHORT = 8'd6;

    reg [7:0] st_q;
    reg [7:0] ret_q;

    // ======================================================================
    // The output slot
    // ======================================================================
    reg [31:0] tok_q;
    reg        tv_q;
    assign tok       = tok_q;
    assign tok_valid = tv_q;
    wire ofree = !tv_q || tok_ready;

    // ======================================================================
    // Exp-Golomb and fixed-length reads off the window
    // ======================================================================
    // Two phases a syntax element. In the first (`ph_q` low) everything
    // below is computed from the window and latched; in the second the
    // state acts on the latched values, consumes and emits. So what the
    // machine's decisions depend on starts at a register, and what the
    // window's next value depends on is the latched length: neither path
    // runs through the other. A state that reads nothing takes one cycle.
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
    wire [5:0] lz_c = clz32(win);

    reg        long_ok_q;
    reg [31:0] long_val_q;
    reg [31:0] long_sev_q;
    // codeNum + 1 of a long code: the bit before the lz bits S_LONG2 reads.
    wire [31:0] long_p1;
    reg  [31:0] long_p1_q;
    reg [4:0]  long_n_q;
    reg [5:0]  long_n1_q;

    // The one-cycle ue(v): the lz + 1 bits from the first 1, minus one.
    // The lz + 1 bits from the first 1 are codeNum + 1: one of sixteen
    // fixed fields of the window, `ufK` for lz = K, chosen by lz rather
    // than shifted by an amount computed from it.
    wire        ue_long_c = !long_ok_q && (lz_c > 6'd15);
    // se(v), Table 9-3: codeNum k is (-1)^(k+1) Ceil(k / 2). With p = k + 1
    // that is p / 2 for an even p and -(p / 2) for an odd one, so the value
    // is a halving and a negation of what the window holds, with no
    // subtraction of the one before them. The one-cycle code is at most 17
    // bits, so its arithmetic is 17 bits wide; the long path works out both
    // values in S_LONG2, off this path.
    // Both values for each of the sixteen code lengths, from the window's
    // bits directly, in parallel with the leading-zero count; the count
    // then only selects. A subtraction or a negation after the selection
    // was the longest path.
    wire [16:0] uf0 = {16'd0, win[31:31]};
    wire [16:0] uk0 = uf0 - 17'd1;
    wire [16:0] sk0 = uf0[0] ? (17'd0 - {1'b0, uf0[16:1]}) : {1'b0, uf0[16:1]};
    wire [16:0] uf1 = {15'd0, win[30:29]};
    wire [16:0] uk1 = uf1 - 17'd1;
    wire [16:0] sk1 = uf1[0] ? (17'd0 - {1'b0, uf1[16:1]}) : {1'b0, uf1[16:1]};
    wire [16:0] uf2 = {14'd0, win[29:27]};
    wire [16:0] uk2 = uf2 - 17'd1;
    wire [16:0] sk2 = uf2[0] ? (17'd0 - {1'b0, uf2[16:1]}) : {1'b0, uf2[16:1]};
    wire [16:0] uf3 = {13'd0, win[28:25]};
    wire [16:0] uk3 = uf3 - 17'd1;
    wire [16:0] sk3 = uf3[0] ? (17'd0 - {1'b0, uf3[16:1]}) : {1'b0, uf3[16:1]};
    wire [16:0] uf4 = {12'd0, win[27:23]};
    wire [16:0] uk4 = uf4 - 17'd1;
    wire [16:0] sk4 = uf4[0] ? (17'd0 - {1'b0, uf4[16:1]}) : {1'b0, uf4[16:1]};
    wire [16:0] uf5 = {11'd0, win[26:21]};
    wire [16:0] uk5 = uf5 - 17'd1;
    wire [16:0] sk5 = uf5[0] ? (17'd0 - {1'b0, uf5[16:1]}) : {1'b0, uf5[16:1]};
    wire [16:0] uf6 = {10'd0, win[25:19]};
    wire [16:0] uk6 = uf6 - 17'd1;
    wire [16:0] sk6 = uf6[0] ? (17'd0 - {1'b0, uf6[16:1]}) : {1'b0, uf6[16:1]};
    wire [16:0] uf7 = {9'd0, win[24:17]};
    wire [16:0] uk7 = uf7 - 17'd1;
    wire [16:0] sk7 = uf7[0] ? (17'd0 - {1'b0, uf7[16:1]}) : {1'b0, uf7[16:1]};
    wire [16:0] uf8 = {8'd0, win[23:15]};
    wire [16:0] uk8 = uf8 - 17'd1;
    wire [16:0] sk8 = uf8[0] ? (17'd0 - {1'b0, uf8[16:1]}) : {1'b0, uf8[16:1]};
    wire [16:0] uf9 = {7'd0, win[22:13]};
    wire [16:0] uk9 = uf9 - 17'd1;
    wire [16:0] sk9 = uf9[0] ? (17'd0 - {1'b0, uf9[16:1]}) : {1'b0, uf9[16:1]};
    wire [16:0] uf10 = {6'd0, win[21:11]};
    wire [16:0] uk10 = uf10 - 17'd1;
    wire [16:0] sk10 = uf10[0] ? (17'd0 - {1'b0, uf10[16:1]}) : {1'b0, uf10[16:1]};
    wire [16:0] uf11 = {5'd0, win[20:9]};
    wire [16:0] uk11 = uf11 - 17'd1;
    wire [16:0] sk11 = uf11[0] ? (17'd0 - {1'b0, uf11[16:1]}) : {1'b0, uf11[16:1]};
    wire [16:0] uf12 = {4'd0, win[19:7]};
    wire [16:0] uk12 = uf12 - 17'd1;
    wire [16:0] sk12 = uf12[0] ? (17'd0 - {1'b0, uf12[16:1]}) : {1'b0, uf12[16:1]};
    wire [16:0] uf13 = {3'd0, win[18:5]};
    wire [16:0] uk13 = uf13 - 17'd1;
    wire [16:0] sk13 = uf13[0] ? (17'd0 - {1'b0, uf13[16:1]}) : {1'b0, uf13[16:1]};
    wire [16:0] uf14 = {2'd0, win[17:3]};
    wire [16:0] uk14 = uf14 - 17'd1;
    wire [16:0] sk14 = uf14[0] ? (17'd0 - {1'b0, uf14[16:1]}) : {1'b0, uf14[16:1]};
    wire [16:0] uf15 = {1'd0, win[16:1]};
    wire [16:0] uk15 = uf15 - 17'd1;
    wire [16:0] sk15 = uf15[0] ? (17'd0 - {1'b0, uf15[16:1]}) : {1'b0, uf15[16:1]};
    reg  [16:0] ue_sel, se_sel;
    always @* begin
        case (lz_c[3:0])
        4'd0: begin ue_sel = uk0; se_sel = sk0; end
        4'd1: begin ue_sel = uk1; se_sel = sk1; end
        4'd2: begin ue_sel = uk2; se_sel = sk2; end
        4'd3: begin ue_sel = uk3; se_sel = sk3; end
        4'd4: begin ue_sel = uk4; se_sel = sk4; end
        4'd5: begin ue_sel = uk5; se_sel = sk5; end
        4'd6: begin ue_sel = uk6; se_sel = sk6; end
        4'd7: begin ue_sel = uk7; se_sel = sk7; end
        4'd8: begin ue_sel = uk8; se_sel = sk8; end
        4'd9: begin ue_sel = uk9; se_sel = sk9; end
        4'd10: begin ue_sel = uk10; se_sel = sk10; end
        4'd11: begin ue_sel = uk11; se_sel = sk11; end
        4'd12: begin ue_sel = uk12; se_sel = sk12; end
        4'd13: begin ue_sel = uk13; se_sel = sk13; end
        4'd14: begin ue_sel = uk14; se_sel = sk14; end
        4'd15: begin ue_sel = uk15; se_sel = sk15; end
        default: begin ue_sel = uk0; se_sel = sk0; end
        endcase
    end
    wire [31:0] uev_c   = long_ok_q ? long_val_q : {15'd0, ue_sel};
    wire [5:0]  uelen_c = long_ok_q ? 6'd0 : {lz_c[4:0], 1'b1};
    wire [31:0] sev_c   = long_ok_q ? long_sev_q : {{15{se_sel[16]}}, se_sel};

    // The latched decode.
    reg [5:0]  lz;
    reg        ue_long;
    reg [31:0] uev;
    reg [31:0] sev;
    reg [31:0] uval;
    // S_LONG2 reads the 1 and the lz bits after it: codeNum + 1 as it stands.
    assign long_p1 = uval;
    reg        w31_q;
    reg [2:0]  wrem_q;
    reg        ph_q;
    // A signed value fits n bits.
    function fits;
        input [31:0] v;
        input [5:0]  n;
        reg   [31:0] hi;
        begin
            hi = $signed(v) >>> (n - 6'd1);
            fits = (hi == 32'd0) || (hi == 32'hFFFF_FFFF);
        end
    endfunction

    // ======================================================================
    // Parameter sets
    // ======================================================================
    // Staging, while one is parsed.
    reg [7:0]  s_prof, s_lvl;
    reg [4:0]  s_id;
    reg [1:0]  s_cf;
    reg [2:0]  s_bdy, s_bdc;
    reg        s_scal, s_bad;
    reg [4:0]  s_lmfn;
    reg [1:0]  s_poct;
    reg [4:0]  s_lmpoc;
    reg        s_dpoaz;
    reg [4:0]  s_maxref;
    reg        s_gaps;
    reg [11:0] s_w, s_h;
    reg        s_fmo, s_d8;
    reg [23:0] s_crop;
    reg [1:0]  crop_i_q;
    reg [7:0]  cyc_q;

    reg [7:0]  p_id;
    reg [4:0]  p_sps;
    reg        p_cabac, p_bfpoc, p_fmo;
    reg [4:0]  p_l0, p_l1;
    reg        p_wp;
    reg [1:0]  p_wb;
    reg [5:0]  p_qp;
    reg [4:0]  p_cqp, p_cqp2;
    reg        p_dfc, p_cip, p_rpc, p_t8, p_scal, p_bad;

    // Scaling list walk: which list, how far into it, and the running
    // nextScale/lastScale of 7.3.2.1.1.1.
    reg [3:0]  scl_i_q;
    reg [3:0]  scl_n_q;
    reg [6:0]  scl_j_q;
    reg [7:0]  scl_last_q;
    reg        scl_pps_q;

    // The stores. 95 bits by 32 SPS ids, 43 by 256 PPS ids; a valid bit
    // per id in registers, so the memories need no reset.
    localparam SPSW = 95, PPSW = 43;
    reg [SPSW-1:0] sps_mem [0:31];
    reg [PPSW-1:0] pps_mem [0:255];
    reg [31:0]     sps_valid_q;
    reg [255:0]    pps_valid_q;
    reg [4:0]      sps_ra_q;
    reg [7:0]      pps_ra_q;
    reg [SPSW-1:0] sps_rd_q;
    reg [PPSW-1:0] pps_rd_q;
    reg            sps_we, pps_we;

    wire [SPSW-1:0] sps_wd = {s_bad, s_scal, s_prof, s_lvl, s_cf, s_bdy, s_bdc, s_lmfn,
                              s_poct, s_lmpoc, s_dpoaz, s_maxref, s_gaps, s_w, s_h,
                              s_fmo, s_d8, s_crop};
    wire [PPSW-1:0] pps_wd = {p_bad, p_sps, p_cabac, p_bfpoc, p_fmo, p_l0, p_l1, p_wp,
                              p_wb, p_qp, p_cqp, p_dfc, p_cip, p_rpc, p_t8, p_scal, p_cqp2};

    always @(posedge clk) begin
        if (sps_we)
            sps_mem[s_id] <= sps_wd;
        sps_rd_q <= sps_mem[sps_ra_q];
    end
    always @(posedge clk) begin
        if (pps_we)
            pps_mem[p_id] <= pps_wd;
        pps_rd_q <= pps_mem[pps_ra_q];
    end

    // The active set, latched at each slice.
    reg        a_bad, a_scal;
    reg [7:0]  a_prof, a_lvl;
    reg [1:0]  a_cf;
    reg [2:0]  a_bdy, a_bdc;
    reg [4:0]  a_lmfn;
    reg [1:0]  a_poct;
    reg [4:0]  a_lmpoc;
    reg        a_dpoaz;
    reg [4:0]  a_maxref;
    reg        a_gaps;
    reg [11:0] a_w, a_h;
    reg        a_fmo, a_d8;
    reg [23:0] a_crop;
    reg        ap_bad;
    reg [4:0]  ap_sps;
    reg        ap_cabac, ap_bfpoc, ap_fmo;
    reg [4:0]  ap_l0, ap_l1;
    reg        ap_wp;
    reg [1:0]  ap_wb;
    reg [5:0]  ap_qp;
    reg [4:0]  ap_cqp, ap_cqp2;
    reg        ap_dfc, ap_cip, ap_rpc, ap_t8, ap_scal;

    // ======================================================================
    // Slice state
    // ======================================================================
    reg [1:0]  nri_q;
    reg        idr_q;
    reg [15:0] first_q;
    reg [2:0]  stype_q;          // slice_type % 5
    reg [7:0]  pps_id_q;
    reg [15:0] fn_q;
    reg [1:0]  npoc_q;
    reg [23:0] poc0_q, poc1_q;   // {which[1:0], value[21:0]}
    reg [3:0]  l0_q;
    reg [4:0]  rplm_cnt_unused;
    reg [4:0]  pwt_i_q;
    reg        pwt_j_q;
    reg [7:0]  pwt_w_q;
    reg [2:0]  mmco_op_q;
    reg [15:0] mmco_x_q;
    reg [5:0]  sqp_q;
    reg [1:0]  dfi_q;
    reg [7:0]  fa_q, fb_q;
    reg [7:0]  err_code_q;
    reg [15:0] err_detail_q;

    wire is_p = (stype_q == 3'd0);

    // ======================================================================
    // Macroblock state
    // ======================================================================
    reg [6:0]  mbx_q;
    reg [11:0] mby_q;
    reg [15:0] addr_q;
    reg [15:0] rem_q;            // first_mb_in_slice, divided down
    reg [7:0]  cnt_q;            // macroblocks since the slice began, saturating
    reg        pic_done_q;
    reg        in_skip_q;
    reg [15:0] skip_q;
    reg [5:0]  qp_q;
    reg [4:0]  kind_q;
    reg [1:0]  i16m_q;
    reg [5:0]  cbp_q;
    reg [1:0]  cpm_q;
    reg        t8_q;
    reg [7:0]  sub_q;            // four sub_mb_types, two bits each, [1:0] first
    reg [1:0]  part_q;           // mbPartIdx
    reg [1:0]  subp_q;           // subMbPartIdx
    reg        comp_q;
    reg [4:0]  ip_q;             // intra pred block counter
    reg [6:0]  pcm_q;
    reg [4:0]  res_q;

    wire is_intra = (kind_q == K_INXN) || (kind_q == K_I16) || (kind_q == K_PCM);
    wire avail_a  = (mbx_q != 7'd0) && (cnt_q != 8'd0);
    wire avail_b  = ({4'd0, cnt_q} >= a_w);
    wire last_mb  = ({5'd0, mbx_q} == a_w - 12'd1) && (mby_q == a_h - 12'd1);

    // The current macroblock's context, in raster order of its 4x4 blocks.
    reg [79:0] tcy_v;
    reg [19:0] tcb_v;
    reg [19:0] tcr_v;
    reg [63:0] md_v;
    // The macroblock to the left: its right column.
    reg [19:0] ltcy_v;
    reg [9:0]  ltcb_v;
    reg [9:0]  ltcr_v;
    reg [15:0] lmd_v;
    reg       linter_q;

    // Element selects and replacements on the packed context vectors,
    // spelt as case statements: a part-select at a computed offset is a
    // multiplier before it is a multiplexer.
    function [4:0] pick5x16;
        input [79:0] v;
        input [3:0] i;
        begin
            case (i)
            4'd0: pick5x16 = v[4:0];
            4'd1: pick5x16 = v[9:5];
            4'd2: pick5x16 = v[14:10];
            4'd3: pick5x16 = v[19:15];
            4'd4: pick5x16 = v[24:20];
            4'd5: pick5x16 = v[29:25];
            4'd6: pick5x16 = v[34:30];
            4'd7: pick5x16 = v[39:35];
            4'd8: pick5x16 = v[44:40];
            4'd9: pick5x16 = v[49:45];
            4'd10: pick5x16 = v[54:50];
            4'd11: pick5x16 = v[59:55];
            4'd12: pick5x16 = v[64:60];
            4'd13: pick5x16 = v[69:65];
            4'd14: pick5x16 = v[74:70];
            4'd15: pick5x16 = v[79:75];
            default: pick5x16 = 5'd0;
            endcase
        end
    endfunction
    function [3:0] pick4x16;
        input [63:0] v;
        input [3:0] i;
        begin
            case (i)
            4'd0: pick4x16 = v[3:0];
            4'd1: pick4x16 = v[7:4];
            4'd2: pick4x16 = v[11:8];
            4'd3: pick4x16 = v[15:12];
            4'd4: pick4x16 = v[19:16];
            4'd5: pick4x16 = v[23:20];
            4'd6: pick4x16 = v[27:24];
            4'd7: pick4x16 = v[31:28];
            4'd8: pick4x16 = v[35:32];
            4'd9: pick4x16 = v[39:36];
            4'd10: pick4x16 = v[43:40];
            4'd11: pick4x16 = v[47:44];
            4'd12: pick4x16 = v[51:48];
            4'd13: pick4x16 = v[55:52];
            4'd14: pick4x16 = v[59:56];
            4'd15: pick4x16 = v[63:60];
            default: pick4x16 = 4'd0;
            endcase
        end
    endfunction
    function [4:0] pick5x4;
        input [19:0] v;
        input [1:0] i;
        begin
            case (i)
            2'd0: pick5x4 = v[4:0];
            2'd1: pick5x4 = v[9:5];
            2'd2: pick5x4 = v[14:10];
            2'd3: pick5x4 = v[19:15];
            default: pick5x4 = 5'd0;
            endcase
        end
    endfunction
    function [4:0] pick5x2;
        input [9:0] v;
        input [0:0] i;
        begin
            case (i)
            1'd0: pick5x2 = v[4:0];
            1'd1: pick5x2 = v[9:5];
            default: pick5x2 = 5'd0;
            endcase
        end
    endfunction
    function [3:0] pick4x4;
        input [15:0] v;
        input [1:0] i;
        begin
            case (i)
            2'd0: pick4x4 = v[3:0];
            2'd1: pick4x4 = v[7:4];
            2'd2: pick4x4 = v[11:8];
            2'd3: pick4x4 = v[15:12];
            default: pick4x4 = 4'd0;
            endcase
        end
    endfunction
    function [1:0] pick2x4;
        input [7:0] v;
        input [1:0] i;
        begin
            case (i)
            2'd0: pick2x4 = v[1:0];
            2'd1: pick2x4 = v[3:2];
            2'd2: pick2x4 = v[5:4];
            2'd3: pick2x4 = v[7:6];
            default: pick2x4 = 2'd0;
            endcase
        end
    endfunction
    function [79:0] put5x16;
        input [79:0] v;
        input [3:0] i;
        input [4:0] x;
        begin
            put5x16 = v;
            case (i)
            4'd0: put5x16[4:0] = x;
            4'd1: put5x16[9:5] = x;
            4'd2: put5x16[14:10] = x;
            4'd3: put5x16[19:15] = x;
            4'd4: put5x16[24:20] = x;
            4'd5: put5x16[29:25] = x;
            4'd6: put5x16[34:30] = x;
            4'd7: put5x16[39:35] = x;
            4'd8: put5x16[44:40] = x;
            4'd9: put5x16[49:45] = x;
            4'd10: put5x16[54:50] = x;
            4'd11: put5x16[59:55] = x;
            4'd12: put5x16[64:60] = x;
            4'd13: put5x16[69:65] = x;
            4'd14: put5x16[74:70] = x;
            4'd15: put5x16[79:75] = x;
            default: ;
            endcase
        end
    endfunction
    function [63:0] put4x16;
        input [63:0] v;
        input [3:0] i;
        input [3:0] x;
        begin
            put4x16 = v;
            case (i)
            4'd0: put4x16[3:0] = x;
            4'd1: put4x16[7:4] = x;
            4'd2: put4x16[11:8] = x;
            4'd3: put4x16[15:12] = x;
            4'd4: put4x16[19:16] = x;
            4'd5: put4x16[23:20] = x;
            4'd6: put4x16[27:24] = x;
            4'd7: put4x16[31:28] = x;
            4'd8: put4x16[35:32] = x;
            4'd9: put4x16[39:36] = x;
            4'd10: put4x16[43:40] = x;
            4'd11: put4x16[47:44] = x;
            4'd12: put4x16[51:48] = x;
            4'd13: put4x16[55:52] = x;
            4'd14: put4x16[59:56] = x;
            4'd15: put4x16[63:60] = x;
            default: ;
            endcase
        end
    endfunction
    function [19:0] put5x4;
        input [19:0] v;
        input [1:0] i;
        input [4:0] x;
        begin
            put5x4 = v;
            case (i)
            2'd0: put5x4[4:0] = x;
            2'd1: put5x4[9:5] = x;
            2'd2: put5x4[14:10] = x;
            2'd3: put5x4[19:15] = x;
            default: ;
            endcase
        end
    endfunction
    function [7:0] put2x4;
        input [7:0] v;
        input [1:0] i;
        input [1:0] x;
        begin
            put2x4 = v;
            case (i)
            2'd0: put2x4[1:0] = x;
            2'd1: put2x4[3:2] = x;
            2'd2: put2x4[5:4] = x;
            2'd3: put2x4[7:6] = x;
            default: ;
            endcase
        end
    endfunction

    // The line buffer: one word per macroblock column, its bottom row.
    //   [56] inter, [55:40] modes x3..x0, [39:30] Cr tc x1,x0,
    //   [29:20] Cb tc x1,x0, [19:0] luma tc x3..x0
    localparam LBW = 57;
    reg [LBW-1:0] lb_mem [0:MAX_W_MBS-1];
    reg [LBW-1:0] lb_q;
    wire          lb_re, lb_we;
    wire [LBW-1:0] lb_wd = {(kind_q >= K_P16), md_v[60 +: 4], md_v[56 +: 4], md_v[52 +: 4], md_v[48 +: 4],
                            tcr_v[15 +: 5], tcr_v[10 +: 5], tcb_v[15 +: 5], tcb_v[10 +: 5],
                            tcy_v[75 +: 5], tcy_v[70 +: 5], tcy_v[65 +: 5], tcy_v[60 +: 5]};
    always @(posedge clk) begin
        if (lb_we)
            lb_mem[mbx_q] <= lb_wd;
        if (lb_re)
            lb_q <= lb_mem[mbx_q];
    end
    wire       tinter = lb_q[56];
    function [3:0] tmd;   input [1:0] x; begin tmd  = pick4x4(lb_q[55:40], x); end endfunction
    function [4:0] ttcr;  input       x; begin ttcr = pick5x2(lb_q[39:30], x); end endfunction
    function [4:0] ttcb;  input       x; begin ttcb = pick5x2(lb_q[29:20], x); end endfunction
    function [4:0] ttcy;  input [1:0] x; begin ttcy = pick5x4(lb_q[19:0], x); end endfunction

    // ======================================================================
    // Neighbours: nC (9.2.1) and the predicted intra mode (8.3.1.1)
    // ======================================================================
    // The luma block the residual walk or the intra walk is on.
    wire [3:0] rblk   = res_q[3:0] - 4'd1;           // res_q 1..16
    wire [3:0] iblk   = t8_q ? {ip_q[1:0], 2'b00} : ip_q[3:0];
    // The intra walk and the residual walk each have their own block, so
    // that neither neighbour lookup waits on a decode of the state.
    wire [3:0] lblk   = iblk;
    wire [1:0] lx     = {lblk[2], lblk[0]};
    wire [1:0] ly     = {lblk[3], lblk[1]};
    wire [3:0] lr     = {ly, lx};
    wire [3:0] rbk    = (res_q == 5'd0) ? 4'd0 : rblk;
    wire [1:0] rx     = {rbk[2], rbk[0]};
    wire [1:0] ry     = {rbk[3], rbk[1]};
    wire [3:0] rr     = {ry, rx};

    wire       na_ok  = (rx != 2'd0) || avail_a;
    wire       nb_ok  = (ry != 2'd0) || avail_b;
    wire [4:0] na     = (rx != 2'd0) ? pick5x16(tcy_v, rr - 4'd1) : pick5x4(ltcy_v, ry);
    wire [4:0] nbv    = (ry != 2'd0) ? pick5x16(tcy_v, rr - 4'd4) : ttcy(rx);
    wire [5:0] nsum   = {1'b0, na} + {1'b0, nbv} + 6'd1;
    wire [4:0] nc_l   = (na_ok && nb_ok) ? nsum[5:1] : na_ok ? na : nb_ok ? nbv : 5'd0;

    // Chroma: res_q 19..22 Cb, 23..26 Cr.
    wire       ccr    = (res_q >= 5'd23);
    wire [1:0] cidx   = ccr ? (res_q - 5'd23) : (res_q - 5'd19);
    wire       cx     = cidx[0];
    wire       cy     = cidx[1];
    wire       nca_ok = cx || avail_a;
    wire       ncb_ok = cy || avail_b;
    wire [4:0] nca    = cx ? (ccr ? pick5x4(tcr_v, {cy, 1'b0}) : pick5x4(tcb_v, {cy, 1'b0}))
                           : (ccr ? pick5x2(ltcr_v, cy) : pick5x2(ltcb_v, cy));
    wire [4:0] ncb    = cy ? (ccr ? pick5x4(tcr_v, {1'b0, cx}) : pick5x4(tcb_v, {1'b0, cx}))
                           : (ccr ? ttcr(cx) : ttcb(cx));
    wire [5:0] ncsum  = {1'b0, nca} + {1'b0, ncb} + 6'd1;
    wire [4:0] nc_c   = (nca_ok && ncb_ok) ? ncsum[5:1] : nca_ok ? nca : ncb_ok ? ncb : 5'd0;

    // The Table 9-5 column for an nC: [0,2) [2,4) [4,8) [8,..).
    wire [2:0] nc_l_cls = (nc_l < 5'd2) ? 3'd0 : (nc_l < 5'd4) ? 3'd1 : (nc_l < 5'd8) ? 3'd2 : 3'd3;
    wire [2:0] nc_c_cls = (nc_c < 5'd2) ? 3'd0 : (nc_c < 5'd4) ? 3'd1 : (nc_c < 5'd8) ? 3'd2 : 3'd3;

    // Intra: dcPredModePredictedFlag, then Min(A, B).
    wire       dc_pred = ((lx == 2'd0) && (!avail_a || (linter_q && ap_cip))) ||
                         ((ly == 2'd0) && (!avail_b || (tinter && ap_cip)));
    wire [3:0] ma     = (lx != 2'd0) ? pick4x16(md_v, lr - 4'd1) : pick4x4(lmd_v, ly);
    wire [3:0] mb     = (ly != 2'd0) ? pick4x16(md_v, lr - 4'd4) : tmd(lx);
    wire [3:0] mpred  = dc_pred ? 4'd2 : ((ma < mb) ? ma : mb);
    wire [2:0] mrem   = wrem_q;
    reg  [3:0] mpred_q;
    wire [3:0] mfinal = w31_q ? mpred_q : (({1'b0, mrem} < mpred_q) ? {1'b0, mrem} : {1'b0, mrem} + 4'd1);

    // ======================================================================
    // Residual walk: which block res_q names, and whether it is coded
    // ======================================================================
    //   0       Intra16x16DCLevel
    //   1..16   luma 4x4 blocks 0..15 (AC for I_16x16)
    //   17, 18  chroma DC Cb, Cr
    //   19..26  chroma AC Cb 0..3, Cr 0..3
    reg        r_coded;
    reg  [4:0] rq_nc;
    reg        rq_dc;
    wire [2:0] rq_cls = rq_dc ? 3'd4 :
                        (rq_nc < 5'd2) ? 3'd0 : (rq_nc < 5'd4) ? 3'd1 : (rq_nc < 5'd8) ? 3'd2 : 3'd3;
    reg  [4:0] r_nc;
    reg  [1:0] rq_kind;
    reg  [3:0] rq_cat;
    reg  [3:0] rq_blk;
    reg [2:0]  r_cls;
    reg [1:0]  r_kind;
    reg [3:0]  r_cat;
    reg [3:0]  r_blk;
    always @* begin
        r_coded = 1'b0;
        r_cls   = 3'd0;
        r_nc    = nc_l;
        r_kind  = 2'd0;
        r_cat   = 4'd2;
        r_blk   = rblk;
        if (res_q == 5'd0) begin
            r_coded = (kind_q == K_I16);
            r_cat   = 4'd0;
            r_blk   = 4'd0;
        end else if (res_q <= 5'd16) begin
            r_coded = cbp_q[rblk[3:2]];
            r_kind  = (kind_q == K_I16) ? 2'd1 : 2'd0;
            r_cat   = (kind_q == K_I16) ? 4'd1 : 4'd2;
        end else if (res_q <= 5'd18) begin
            r_coded = (cbp_q[5:4] != 2'd0);
            r_cls   = 3'd4;
            r_kind  = 2'd2;
            r_cat   = (res_q == 5'd17) ? 4'd3 : 4'd4;
            r_blk   = 4'd0;
        end else begin
            r_coded = (cbp_q[5:4] == 2'd2);
            r_nc    = nc_c;
            r_kind  = 2'd1;
            r_cat   = ccr ? 4'd6 : 4'd5;
            r_blk   = {2'd0, cidx};
        end
    end

    wire        cav_busy, cav_done, cav_err, cav_tok_we;
    wire [5:0]  cav_consume;
    wire [31:0] cav_tok;
    wire [4:0]  cav_tc;
    wire        cav_start;
    wire        cav_go, cav_latch, cav_reads, cav_hold;
    wire [5:0]  cav_dlen;

    h264_cavlc u_cav (
        .clk(clk), .rst_n(rst_n),
        .start(cav_start), .abort(st_q == S_ERR), .cls(rq_cls), .kind(rq_kind), .cat(rq_cat), .blk(rq_blk),
        .win(win), .latch(cav_latch), .go(cav_go), .reads(cav_reads), .hold(cav_hold), .dlen(cav_dlen), .consume(cav_consume),
        .tok_we(cav_tok_we), .tok(cav_tok),
        .busy(cav_busy), .done(cav_done), .err(cav_err), .tc_out(cav_tc)
    );

    // coded_block_pattern, Table 9-4 (a).
    wire       cbp_valid;
    wire [5:0] cbp_val;
    wire       u0, u1, u2, u3, u4, u5;
    wire [4:0] u6, u6b, u7;
    wire [1:0] u8;
    wire [3:0] u9, u10, u11, u12;
    h264_vlc u_cbp (
        .ct_cls(3'd0), .ct_bits(16'd0), .ct_valid(u0), .ct_len(u6), .ct_len1(u6b), .ct_t1(u8), .ct_tc(u7),
        .tz_dc(1'b0), .tz_tc(4'd0), .tz_bits(9'd0), .tz_valid(u1), .tz_len(u9), .tz_val(u10),
        .rb_zl(3'd0), .rb_bits(11'd0), .rb_valid(u2), .rb_len(u11), .rb_val(u12),
        .cbp_code(uev[5:0]), .cbp_inter(kind_q != K_INXN),
        .cbp_valid(cbp_valid), .cbp_val(cbp_val)
    );
    wire cbp_ok = cbp_valid && (uev < 32'd48);

    // ======================================================================
    // mb_type, Tables 7-11 and 7-13
    // ======================================================================
    // The intra value: mb_type in an I slice, mb_type - 5 in a P slice.
    wire [31:0] imt = is_p ? (uev - 32'd5) : uev;
    wire        mt_intra = !is_p || (uev >= 32'd5);
    wire        mt_ok = is_p ? (uev <= 32'd30) : (uev <= 32'd25);
    wire [4:0]  i16v = imt[4:0] - 5'd1;      // 0..23
    reg  [4:0]  mt_kind;
    reg  [1:0]  mt_i16m;
    reg  [5:0]  mt_cbp;
    always @* begin
        mt_kind = K_INXN;
        mt_i16m = 2'd0;
        mt_cbp  = 6'd0;
        if (!mt_intra) begin
            case (uev[2:0])
                3'd0:    mt_kind = K_P16;
                3'd1:    mt_kind = K_P168;
                3'd2:    mt_kind = K_P816;
                3'd3:    mt_kind = K_P8;
                default: mt_kind = K_P8R0;
            endcase
        end else if (imt == 32'd0) begin
            mt_kind = K_INXN;
        end else if (imt == 32'd25) begin
            mt_kind = K_PCM;
        end else begin
            mt_kind = K_I16;
            mt_i16m = i16v[1:0];
            // Table 7-11: CodedBlockPatternChroma is (mb_type - 1) / 4 % 3,
            // and CodedBlockPatternLuma is 15 from mb_type 13 on.
            mt_cbp[3:0] = (i16v >= 5'd12) ? 4'hF : 4'h0;
            if (i16v >= 5'd12)
                mt_cbp[5:4] = (i16v >= 5'd20) ? 2'd2 : (i16v >= 5'd16) ? 2'd1 : 2'd0;
            else
                mt_cbp[5:4] = (i16v >= 5'd8) ? 2'd2 : (i16v >= 5'd4) ? 2'd1 : 2'd0;
        end
    end

    // The partition counts of the current inter macroblock.
    wire [1:0] nparts_m1 = (kind_q == K_P16) ? 2'd0 : (kind_q == K_P8 || kind_q == K_P8R0) ? 2'd3 : 2'd1;
    wire [1:0] sub_cur   = pick2x4(sub_q, part_q);
    // NumSubMbPart - 1, Table 7-17: 8x8 one, 8x4 and 4x8 two, 4x4 four.
    wire [1:0] nsub_m1   = (kind_q == K_P8 || kind_q == K_P8R0) ?
                           ((sub_cur == 2'd0) ? 2'd0 : (sub_cur == 2'd3) ? 2'd3 : 2'd1) : 2'd0;

    // QPY after mb_qp_delta, 7.4.5, for 8-bit video.
    wire [6:0] qsum = {1'b0, qp_q} + sev[6:0];
    wire [5:0] qnew = ($signed(qsum) < 0) ? (qsum[5:0] + 6'd52) :
                      (qsum > 7'd51) ? (qsum[5:0] - 6'd52) : qsum[5:0];

    // ======================================================================
    // Per-state reads: what `consume` takes and whether the window is needed
    // ======================================================================
    localparam [2:0] RT_NONE = 3'd0, RT_UE = 3'd1, RT_U = 3'd2, RT_CAV = 3'd3,
                     RT_SPEC = 3'd4;
    reg [2:0] rt;
    reg [5:0] rn;       // bits, for RT_U and RT_SPEC
    reg       need_win;
    reg       need_more;
    always @* begin
        rt        = RT_NONE;
        rn        = 6'd0;
        need_win  = 1'b1;
        need_more = 1'b0;
        case (st_q)
        S_DROP, S_ERR, S_LONG3, S_SH_ACT1, S_SH_ACT2, S_SH_ACT3, S_SH_ACT4,
        S_SPS_STORE, S_PPS_STORE, S_SD_DIV, S_RES, S_RES2:
            need_win = 1'b0;
        S_PPS_MORE, S_AFTER: begin
            need_win  = 1'b0;
            need_more = 1'b1;
        end
        S_LONG1:     begin rt = RT_SPEC; rn = lz; end
        S_LONG2:     begin rt = RT_U; rn = long_n1_q; end
        S_NAL:       begin rt = RT_U; rn = 6'd8; end
        S_SPS_PROF:  begin rt = RT_U; rn = 6'd24; end
        S_SPS_SEP, S_SPS_QPP, S_SPS_SCF, S_SPS_DPOAZ, S_SPS_GAPS, S_SPS_FMO,
        S_SPS_MBAFF, S_SPS_D8, S_SPS_CROPF, S_SCL_FLAG,
        S_PPS_ENT, S_PPS_BFP, S_PPS_WP, S_PPS_DFC, S_PPS_CIP, S_PPS_RPC,
        S_PPS_T8, S_PPS_SCF, S_SH_OVR, S_RPLM_F, S_PWT_LF, S_PWT_CF,
        S_DRPM_A, S_T8A, S_T8B:
            begin rt = RT_U; rn = 6'd1; end
        S_PPS_WB, S_DRPM_IDR:
            begin rt = RT_U; rn = 6'd2; end
        S_SH_FN:     begin rt = RT_U; rn = {1'b0, a_lmfn}; end
        S_SH_POCL:   begin rt = RT_U; rn = {1'b0, a_lmpoc}; end
        S_SPS_ID, S_SPS_CF, S_SPS_BDY, S_SPS_BDC, S_SPS_LMFN, S_SPS_POCT,
        S_SPS_LMPOC, S_SPS_ONRP, S_SPS_OTTB, S_SPS_NCYC, S_SPS_OFR, S_SPS_MAXR,
        S_SPS_W, S_SPS_H, S_SPS_CROP, S_SCL_DELTA,
        S_PPS_ID, S_PPS_SPS, S_PPS_NSG, S_PPS_L0, S_PPS_L1, S_PPS_QP, S_PPS_QS,
        S_PPS_CQP, S_PPS_CQP2,
        S_SH_FIRST, S_SH_TYPE, S_SH_PPS, S_SH_IDR, S_SH_DPB, S_SH_DP0, S_SH_DP1,
        S_SH_RPC, S_SH_L0, S_RPLM_IDC, S_RPLM_V, S_PWT_LD, S_PWT_CD, S_PWT_LW,
        S_PWT_LO, S_PWT_CW, S_PWT_CO, S_MMCO_OP, S_MMCO_X, S_MMCO_Y, S_SH_QPD,
        S_SH_DFI, S_SH_FA, S_SH_FB,
        S_SKIPRUN, S_MB_TYPE, S_SUB, S_CPM, S_MVD, S_CBP, S_QPD:
            rt = RT_UE;
        S_REF:
            if (kind_q == K_P8R0 || l0_q == 4'd0) rt = RT_NONE;
            else if (l0_q == 4'd1) begin rt = RT_U; rn = 6'd1; end
            else rt = RT_UE;
        S_IPRED:     begin rt = RT_U; rn = w31_q ? 6'd1 : 6'd4; end
        S_PCM_ALIGN: begin rt = RT_SPEC; rn = {3'd0, 3'd0 - bitpos}; end
        S_PCM_DATA:  begin rt = RT_U; rn = 6'd24; end
        S_RESW:      rt = RT_CAV;
        default: ;
        endcase
    end

    // Whether this state reads the window, and so takes two phases.
    // `resw_q` is `st_q == S_RESW` kept as a register of its own, because
    // what depends on it -- the CAVLC block's every enable -- is wide.
    reg  resw_q;
    wire reads = resw_q ? cav_reads : (rt != RT_NONE);
    // S_DROP needs nothing, and goes on its own: `drop` is then a decode of
    // the state and nothing else, because it reaches the whole bit reader.
    // A second phase needs only the output slot (the window it decoded is
    // still there: nothing consumed it); a state that reads nothing needs
    // the slot, and `more_rbsp_data()` where it asks.
    wire go = (st_q == S_DROP) ||
              (ofree && (ph_q ? !(cons_cav_q && cav_hold) : (!reads && (!need_more || more_ok))));
    // The first phase of a reading state: the window is there to decode.
    wire latch = reads && !ph_q && win_ok;
    assign cav_go = resw_q && ofree && (ph_q ? !cav_hold : !cav_reads);
    assign cav_latch = latch && resw_q;
    assign cav_start = go && (st_q == S_RES2);
    assign lb_re = go && (st_q == S_MB_START) && (mby_q < a_h);
    assign lb_we = go && (st_q == S_MB_END);

    // What the first phase latches as the second phase's consumption, from
    // the decode being latched: the Exp-Golomb length, or the state's
    // fixed one. For S_RESW it is the CAVLC block's own latched length,
    // which `cav_dlen` carries.
    wire [5:0] want_c = (rt == RT_UE) ? (ue_long_c ? 6'd0 : uelen_c) :
                        (st_q == S_IPRED) ? (win[31] ? 6'd1 : 6'd4) :
                        (st_q == S_LONG1) ? lz_c :
                        (rt == RT_U || rt == RT_SPEC) ? rn : 6'd0;
    reg  [5:0] want_q;
    // A NAL unit that ends before its syntax does: decided in the first
    // phase, from the length being latched. Once the last byte is in,
    // `avail` only falls, and before it is in a full window covers any
    // symbol, so the answer holds into the second phase. The CAVLC block's
    // lengths are its own registers, so its check is made in the second.
    // A NAL unit that ends before its syntax does: the bit reader flags the
    // read that went past its end, and the state after it reports it. The
    // token that read's state emitted, if any, goes out first, from bits
    // that were not there.
    wire overrun = short && (st_q != S_ERR) && (st_q != S_DROP) && (st_q != S_NAL);
    reg        cons_cav_q;
    // Only a reading state's second phase consumes, so `consume` is a
    // function of registers and of the output slot: the bit reader's next
    // window, which is the next symbol's decode, does not wait on the
    // state machine's decode of its own state. Not gated by `overrun`: that
    // cycle fails the NAL unit, and the bits it takes are dropped with the
    // rest.
    always @* consume = (ph_q && ofree && !(cons_cav_q && cav_hold)) ?
                        (cons_cav_q ? cav_dlen : want_q) : 6'd0;
    // One cycle only: a second would throw away a header byte the first kept.
    assign drop = (st_q == S_DROP);

    // The fixed-length value of an RT_U read.
    wire [31:0] uval_c = (rn == 6'd0) ? 32'd0 : (win >> (6'd32 - rn));

    // ======================================================================
    // The machine
    // ======================================================================

    // Reports `code` with `detail` and abandons the NAL unit.
    task fail;
        input [7:0]  code;
        input [15:0] detail;
        begin
            err_code_q   <= code;
            err_detail_q <= detail;
            st_q         <= S_ERR;
            // A failure can land between a state's two phases; the next
            // state must start at its first.
            ph_q         <= 1'b0;
            resw_q       <= 1'b0;
        end
    endtask

    task emit;
        input [7:0]  tag;
        input [23:0] data;
        begin
            tok_q <= {tag, data};
            tv_q  <= 1'b1;
        end
    endtask

    // Before the next element of a slice header that follows the POC
    // fields: the redundant_pic_cnt if the PPS has it, else the tokens.
    task after_poc;
        begin
            if (ap_rpc)
                st_q <= S_SH_RPC;
            else
                st_q <= (first_q == 16'd0) ? S_E_DIMS : S_E_HDR;
        end
    endtask

    // After SLICE_REF: ref_pic_list_modification for a P slice, else on.
    task after_ref;
        begin
            if (is_p)
                st_q <= S_RPLM_F;
            else
                after_rplm;
        end
    endtask

    task after_rplm;
        begin
            if (ap_wp && is_p)
                st_q <= S_PWT_LD;
            else
                after_pwt;
        end
    endtask

    task after_pwt;
        begin
            if (nri_q != 2'd0)
                st_q <= idr_q ? S_DRPM_IDR : S_DRPM_A;
            else
                st_q <= S_SH_QPD;
        end
    endtask

    task after_dblk_fields;
        begin
            st_q <= S_E_QP;
        end
    endtask

    // Clears the current macroblock's context before it is parsed.
    task clear_mb;
        input [4:0] tc;
        begin
            tcy_v <= {16{tc}};
            md_v  <= {16{4'd2}};
            tcb_v <= {4{tc}};
            tcr_v <= {4{tc}};
        end
    endtask

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            st_q <= S_NAL;
            ret_q <= S_NAL;
            tok_q <= 32'd0;
            tv_q <= 1'b0;
            long_ok_q <= 1'b0;
            long_val_q <= 32'd0;
            long_sev_q <= 32'd0;
            long_n_q <= 5'd0;
            long_n1_q <= 6'd0;
            lz <= 6'd0; ue_long <= 1'b0; uev <= 32'd0;
            sev <= 32'd0; uval <= 32'd0; w31_q <= 1'b0; wrem_q <= 3'd0; ph_q <= 1'b0;
            want_q <= 6'd0; cons_cav_q <= 1'b0; resw_q <= 1'b0; mpred_q <= 4'd2;
            long_p1_q <= 32'd0;
            rq_nc <= 5'd0; rq_dc <= 1'b0; rq_kind <= 2'd0; rq_cat <= 4'd0; rq_blk <= 4'd0;
            s_prof <= 8'd0; s_lvl <= 8'd0; s_id <= 5'd0; s_cf <= 2'd1;
            s_bdy <= 3'd0; s_bdc <= 3'd0; s_scal <= 1'b0; s_bad <= 1'b0;
            s_lmfn <= 5'd4; s_poct <= 2'd0; s_lmpoc <= 5'd0; s_dpoaz <= 1'b0;
            s_maxref <= 5'd0; s_gaps <= 1'b0; s_w <= 12'd0; s_h <= 12'd0;
            s_fmo <= 1'b1; s_d8 <= 1'b0; s_crop <= 24'd0; crop_i_q <= 2'd0;
            cyc_q <= 8'd0;
            p_id <= 8'd0; p_sps <= 5'd0; p_cabac <= 1'b0; p_bfpoc <= 1'b0;
            p_fmo <= 1'b0; p_l0 <= 5'd0; p_l1 <= 5'd0; p_wp <= 1'b0; p_wb <= 2'd0;
            p_qp <= 6'd26; p_cqp <= 5'd0; p_cqp2 <= 5'd0; p_dfc <= 1'b0;
            p_cip <= 1'b0; p_rpc <= 1'b0; p_t8 <= 1'b0; p_scal <= 1'b0; p_bad <= 1'b0;
            scl_i_q <= 4'd0; scl_n_q <= 4'd0; scl_j_q <= 7'd0; scl_last_q <= 8'd8;
            scl_pps_q <= 1'b0;
            sps_valid_q <= 32'd0;
            pps_valid_q <= 256'd0;
            sps_ra_q <= 5'd0; pps_ra_q <= 8'd0;
            sps_we <= 1'b0; pps_we <= 1'b0;
            a_bad <= 1'b0; a_scal <= 1'b0; a_prof <= 8'd0; a_lvl <= 8'd0;
            a_cf <= 2'd1; a_bdy <= 3'd0; a_bdc <= 3'd0; a_lmfn <= 5'd4;
            a_poct <= 2'd0; a_lmpoc <= 5'd4; a_dpoaz <= 1'b0; a_maxref <= 5'd0;
            a_gaps <= 1'b0; a_w <= 12'd1; a_h <= 12'd1; a_fmo <= 1'b1; a_d8 <= 1'b0;
            a_crop <= 24'd0;
            ap_bad <= 1'b0; ap_sps <= 5'd0; ap_cabac <= 1'b0; ap_bfpoc <= 1'b0;
            ap_fmo <= 1'b0; ap_l0 <= 5'd0; ap_l1 <= 5'd0; ap_wp <= 1'b0; ap_wb <= 2'd0;
            ap_qp <= 6'd26; ap_cqp <= 5'd0; ap_cqp2 <= 5'd0; ap_dfc <= 1'b0;
            ap_cip <= 1'b0; ap_rpc <= 1'b0; ap_t8 <= 1'b0; ap_scal <= 1'b0;
            nri_q <= 2'd0; idr_q <= 1'b0; first_q <= 16'd0; stype_q <= 3'd0;
            pps_id_q <= 8'd0; fn_q <= 16'd0; npoc_q <= 2'd0; poc0_q <= 24'd0;
            poc1_q <= 24'd0; l0_q <= 4'd0; rplm_cnt_unused <= 5'd0;
            pwt_i_q <= 5'd0; pwt_j_q <= 1'b0; pwt_w_q <= 8'd0;
            mmco_op_q <= 3'd0; mmco_x_q <= 16'd0;
            sqp_q <= 6'd26; dfi_q <= 2'd0; fa_q <= 8'd0; fb_q <= 8'd0;
            err_code_q <= 8'd0; err_detail_q <= 16'd0;
            mbx_q <= 7'd0; mby_q <= 12'd0; addr_q <= 16'd0; rem_q <= 16'd0;
            cnt_q <= 8'd0; pic_done_q <= 1'b0; in_skip_q <= 1'b0; skip_q <= 16'd0;
            qp_q <= 6'd26; kind_q <= K_INXN; i16m_q <= 2'd0; cbp_q <= 6'd0;
            cpm_q <= 2'd0; t8_q <= 1'b0; sub_q <= 8'd0; part_q <= 2'd0;
            subp_q <= 2'd0; comp_q <= 1'b0; ip_q <= 5'd0; pcm_q <= 7'd0;
            res_q <= 5'd0;
            linter_q <= 1'b0;
            tcy_v  <= 80'd0;
            md_v   <= {16{4'd2}};
            tcb_v  <= 20'd0;
            tcr_v  <= 20'd0;
            ltcy_v <= 20'd0;
            lmd_v  <= {4{4'd2}};
            ltcb_v <= 10'd0;
            ltcr_v <= 10'd0;
        end else begin
            if (tv_q && tok_ready)
                tv_q <= 1'b0;
            sps_we <= 1'b0;
            pps_we <= 1'b0;
            if (go && rt == RT_UE && !ue_long)
                long_ok_q <= 1'b0;
            if (latch) begin
                lz      <= lz_c;
                ue_long <= ue_long_c;
                uev     <= uev_c;
                sev     <= sev_c;
                uval    <= uval_c;
                w31_q   <= win[31];
                // The neighbours' modes are settled by the first phase: the
                // block before wrote its mode on its own second phase.
                mpred_q <= mpred;
                want_q  <= want_c;
                cons_cav_q <= (st_q == S_RESW);
                wrem_q  <= win[30:28];
                ph_q    <= 1'b1;
            end
            if (go)
                ph_q <= 1'b0;

            if (overrun) begin
                fail(E_SHORT, addr_q);
            end else if (go && rt == RT_UE && ue_long) begin
                // A code too long for the window: two cycles more.
                ret_q <= st_q;
                st_q  <= S_LONG1;
            end else if (go) begin
                case (st_q)
                // ---------------------------------------------------------
                S_NAL: begin
                    nri_q <= uval[6:5];
                    idr_q <= (uval[4:0] == 5'd5);
                    if (uval[7])
                        st_q <= S_DROP;
                    else case (uval[4:0])
                        5'd7:       st_q <= S_SPS_PROF;
                        5'd8:       st_q <= S_PPS_ID;
                        5'd1, 5'd5: st_q <= S_SH_FIRST;
                        default:    st_q <= S_DROP;
                    endcase
                end
                S_DROP:
                    st_q <= S_NAL;
                S_ERR: begin
                    emit(8'h7F, {err_code_q, err_detail_q});
                    st_q <= S_DROP;
                end
                S_LONG1: begin
                    if (lz == 6'd32) begin
                        fail(E_SYNTAX, addr_q);
                    end else begin
                        long_n_q <= lz[4:0];
                        long_n1_q <= lz + 6'd1;
                        st_q <= S_LONG2;
                    end
                end
                S_LONG2: begin
                    long_p1_q <= long_p1;
                    st_q      <= S_LONG3;
                end
                S_LONG3: begin
                    // codeNum, and the se(v) of it, from codeNum + 1.
                    long_val_q <= long_p1_q - 32'd1;
                    long_sev_q <= long_p1_q[0] ? (32'd0 - {1'b0, long_p1_q[31:1]}) : {1'b0, long_p1_q[31:1]};
                    long_ok_q  <= 1'b1;
                    st_q       <= ret_q;
                end

                // ---------------------------------------------------------
                // Sequence parameter set, 7.3.2.1.1
                S_SPS_PROF: begin
                    s_prof <= uval[23:16];
                    s_lvl  <= uval[7:0];
                    s_cf   <= 2'd1; s_bdy <= 3'd0; s_bdc <= 3'd0;
                    s_scal <= 1'b0; s_bad <= 1'b0; s_lmpoc <= 5'd0; s_dpoaz <= 1'b0;
                    s_crop <= 24'd0; s_fmo <= 1'b1;
                    st_q <= S_SPS_ID;
                end
                S_SPS_ID: begin
                    s_id <= uev[4:0];
                    if (uev > 32'd31)
                        st_q <= S_DROP;
                    else case (s_prof)
                        8'd100, 8'd110, 8'd122, 8'd244, 8'd44, 8'd83, 8'd86,
                        8'd118, 8'd128, 8'd138, 8'd139, 8'd134, 8'd135:
                            st_q <= S_SPS_CF;
                        default:
                            st_q <= S_SPS_LMFN;
                    endcase
                end
                S_SPS_CF: begin
                    s_cf <= uev[1:0];
                    if (uev > 32'd3)
                        st_q <= S_DROP;
                    else begin
                        if (uev != 32'd1)
                            s_bad <= 1'b1;
                        st_q <= (uev == 32'd3) ? S_SPS_SEP : S_SPS_BDY;
                    end
                end
                S_SPS_SEP:
                    st_q <= S_SPS_BDY;
                S_SPS_BDY: begin
                    s_bdy <= uev[2:0];
                    if (uev != 32'd0) s_bad <= 1'b1;
                    if (uev > 32'd6) st_q <= S_DROP; else st_q <= S_SPS_BDC;
                end
                S_SPS_BDC: begin
                    s_bdc <= uev[2:0];
                    if (uev != 32'd0) s_bad <= 1'b1;
                    if (uev > 32'd6) st_q <= S_DROP; else st_q <= S_SPS_QPP;
                end
                S_SPS_QPP: begin
                    // qpprime_y_zero_transform_bypass_flag: lossless, not here.
                    if (uval[0]) s_bad <= 1'b1;
                    st_q <= S_SPS_SCF;
                end
                S_SPS_SCF: begin
                    s_scal <= uval[0];
                    if (uval[0]) begin
                        scl_pps_q <= 1'b0;
                        scl_i_q   <= 4'd0;
                        scl_n_q   <= (s_cf == 2'd3) ? 4'd12 : 4'd8;
                        st_q      <= S_SCL_FLAG;
                    end else
                        st_q <= S_SPS_LMFN;
                end
                S_SPS_LMFN: begin
                    s_lmfn <= uev[4:0] + 5'd4;
                    if (uev > 32'd12) st_q <= S_DROP; else st_q <= S_SPS_POCT;
                end
                S_SPS_POCT: begin
                    s_poct <= uev[1:0];
                    if (uev > 32'd2) st_q <= S_DROP;
                    else if (uev == 32'd0) st_q <= S_SPS_LMPOC;
                    else if (uev == 32'd1) st_q <= S_SPS_DPOAZ;
                    else st_q <= S_SPS_MAXR;
                end
                S_SPS_LMPOC: begin
                    s_lmpoc <= uev[4:0] + 5'd4;
                    if (uev > 32'd12) st_q <= S_DROP; else st_q <= S_SPS_MAXR;
                end
                S_SPS_DPOAZ: begin
                    s_dpoaz <= uval[0];
                    st_q <= S_SPS_ONRP;
                end
                S_SPS_ONRP:
                    st_q <= S_SPS_OTTB;
                S_SPS_OTTB:
                    st_q <= S_SPS_NCYC;
                S_SPS_NCYC: begin
                    cyc_q <= uev[7:0];
                    if (uev > 32'd255) st_q <= S_DROP;
                    else st_q <= (uev == 32'd0) ? S_SPS_MAXR : S_SPS_OFR;
                end
                S_SPS_OFR: begin
                    cyc_q <= cyc_q - 8'd1;
                    if (cyc_q == 8'd1) st_q <= S_SPS_MAXR;
                end
                S_SPS_MAXR: begin
                    s_maxref <= uev[4:0];
                    if (uev > 32'd16) st_q <= S_DROP; else st_q <= S_SPS_GAPS;
                end
                S_SPS_GAPS: begin
                    s_gaps <= uval[0];
                    st_q <= S_SPS_W;
                end
                S_SPS_W: begin
                    s_w <= uev[11:0] + 12'd1;
                    if (uev >= MAX_W_MBS) s_bad <= 1'b1;
                    st_q <= S_SPS_H;
                end
                S_SPS_H: begin
                    s_h <= uev[11:0] + 12'd1;
                    if (uev > 32'd4094) s_bad <= 1'b1;
                    st_q <= S_SPS_FMO;
                end
                S_SPS_FMO: begin
                    s_fmo <= uval[0];
                    if (!uval[0]) s_bad <= 1'b1;
                    st_q <= uval[0] ? S_SPS_D8 : S_SPS_MBAFF;
                end
                S_SPS_MBAFF:
                    st_q <= S_SPS_D8;
                S_SPS_D8: begin
                    s_d8 <= uval[0];
                    st_q <= S_SPS_CROPF;
                end
                S_SPS_CROPF: begin
                    crop_i_q <= 2'd0;
                    st_q <= uval[0] ? S_SPS_CROP : S_SPS_STORE;
                end
                S_SPS_CROP: begin
                    if (uev > 32'd63) s_bad <= 1'b1;
                    s_crop <= {s_crop[17:0], uev[5:0]};
                    crop_i_q <= crop_i_q + 2'd1;
                    if (crop_i_q == 2'd3) st_q <= S_SPS_STORE;
                end
                S_SPS_STORE: begin
                    // The VUI, if any, is not needed: stop here.
                    sps_we <= 1'b1;
                    sps_valid_q[s_id] <= 1'b1;
                    st_q <= S_DROP;
                end

                // ---------------------------------------------------------
                // scaling_list(), 7.3.2.1.1.1: parsed to be skipped.
                S_SCL_FLAG: begin
                    scl_j_q    <= 7'd0;
                    scl_last_q <= 8'd8;
                    if (uval[0])
                        st_q <= S_SCL_DELTA;
                    else begin
                        scl_i_q <= scl_i_q + 4'd1;
                        if (scl_i_q + 4'd1 == scl_n_q)
                            st_q <= scl_pps_q ? S_PPS_CQP2 : S_SPS_LMFN;
                    end
                end
                S_SCL_DELTA: begin
                    // nextScale = (lastScale + delta_scale + 256) % 256; a
                    // zero ends the list's explicit values.
                    if (sev[7:0] + scl_last_q == 8'd0 ||
                        scl_j_q + 7'd1 == ((scl_i_q < 4'd6) ? 7'd16 : 7'd64)) begin
                        scl_i_q <= scl_i_q + 4'd1;
                        if (scl_i_q + 4'd1 == scl_n_q)
                            st_q <= scl_pps_q ? S_PPS_CQP2 : S_SPS_LMFN;
                        else
                            st_q <= S_SCL_FLAG;
                    end else begin
                        scl_last_q <= sev[7:0] + scl_last_q;
                        scl_j_q    <= scl_j_q + 7'd1;
                    end
                end

                // ---------------------------------------------------------
                // Picture parameter set, 7.3.2.2
                S_PPS_ID: begin
                    p_id <= uev[7:0];
                    p_bad <= 1'b0; p_fmo <= 1'b0; p_t8 <= 1'b0; p_scal <= 1'b0;
                    if (uev > 32'd255) st_q <= S_DROP; else st_q <= S_PPS_SPS;
                end
                S_PPS_SPS: begin
                    p_sps <= uev[4:0];
                    if (uev > 32'd31) st_q <= S_DROP; else st_q <= S_PPS_ENT;
                end
                S_PPS_ENT: begin p_cabac <= uval[0]; st_q <= S_PPS_BFP; end
                S_PPS_BFP: begin p_bfpoc <= uval[0]; st_q <= S_PPS_NSG; end
                S_PPS_NSG: begin
                    if (uev != 32'd0) begin
                        // Slice groups: recorded, and refused at activation.
                        p_fmo <= 1'b1;
                        st_q  <= S_PPS_STORE;
                    end else
                        st_q <= S_PPS_L0;
                end
                S_PPS_L0: begin
                    p_l0 <= uev[4:0];
                    if (uev > 32'd31) st_q <= S_DROP; else st_q <= S_PPS_L1;
                end
                S_PPS_L1: begin
                    p_l1 <= uev[4:0];
                    if (uev > 32'd31) st_q <= S_DROP; else st_q <= S_PPS_WP;
                end
                S_PPS_WP: begin p_wp <= uval[0]; st_q <= S_PPS_WB; end
                S_PPS_WB: begin p_wb <= uval[1:0]; st_q <= S_PPS_QP; end
                S_PPS_QP: begin
                    // pic_init_qp_minus26: -26..25 for 8-bit video.
                    p_qp <= sev[5:0] + 6'd26;
                    if (!fits(sev, 6'd6) || $signed(sev) < -26 || $signed(sev) > 25)
                        p_bad <= 1'b1;
                    st_q <= S_PPS_QS;
                end
                S_PPS_QS:  st_q <= S_PPS_CQP;
                S_PPS_CQP: begin
                    p_cqp  <= sev[4:0];
                    p_cqp2 <= sev[4:0];
                    if ($signed(sev) < -12 || $signed(sev) > 12) p_bad <= 1'b1;
                    st_q <= S_PPS_DFC;
                end
                S_PPS_DFC: begin p_dfc <= uval[0]; st_q <= S_PPS_CIP; end
                S_PPS_CIP: begin p_cip <= uval[0]; st_q <= S_PPS_RPC; end
                S_PPS_RPC: begin p_rpc <= uval[0]; st_q <= S_PPS_MORE; end
                S_PPS_MORE:
                    st_q <= more ? S_PPS_T8 : S_PPS_STORE;
                S_PPS_T8:  begin p_t8 <= uval[0]; st_q <= S_PPS_SCF; end
                S_PPS_SCF: begin
                    p_scal <= uval[0];
                    if (uval[0]) begin
                        // 6 + 2 * transform_8x8_mode_flag lists for 4:2:0;
                        // 4:4:4's six 8x8 lists are refused with the SPS.
                        scl_pps_q <= 1'b1;
                        scl_i_q   <= 4'd0;
                        scl_n_q   <= p_t8 ? 4'd8 : 4'd6;
                        st_q      <= S_SCL_FLAG;
                    end else
                        st_q <= S_PPS_CQP2;
                end
                S_PPS_CQP2: begin
                    p_cqp2 <= sev[4:0];
                    if ($signed(sev) < -12 || $signed(sev) > 12) p_bad <= 1'b1;
                    st_q <= S_PPS_STORE;
                end
                S_PPS_STORE: begin
                    pps_we <= 1'b1;
                    pps_valid_q[p_id] <= 1'b1;
                    st_q <= S_DROP;
                end

                // ---------------------------------------------------------
                // Slice header, 7.3.3
                S_SH_FIRST: begin
                    first_q <= uev[15:0];
                    if (uev > 32'd65535) fail(E_SYNTAX, 16'd0); else st_q <= S_SH_TYPE;
                end
                S_SH_TYPE: begin
                    stype_q <= (uev >= 32'd5) ? (uev[2:0] - 3'd5) : uev[2:0];
                    if (uev > 32'd9) fail(E_SYNTAX, 16'd0); else st_q <= S_SH_PPS;
                end
                S_SH_PPS: begin
                    pps_id_q <= uev[7:0];
                    pps_ra_q <= uev[7:0];
                    if (uev > 32'd255) fail(E_SYNTAX, 16'd0); else st_q <= S_SH_ACT1;
                end
                S_SH_ACT1:
                    st_q <= S_SH_ACT2;
                S_SH_ACT2: begin
                    {ap_bad, ap_sps, ap_cabac, ap_bfpoc, ap_fmo, ap_l0, ap_l1, ap_wp,
                     ap_wb, ap_qp, ap_cqp, ap_dfc, ap_cip, ap_rpc, ap_t8, ap_scal, ap_cqp2} <= pps_rd_q;
                    sps_ra_q <= pps_rd_q[41:37];
                    if (!pps_valid_q[pps_id_q])
                        fail(E_MISSING, 16'd0);
                    else
                        st_q <= S_SH_ACT3;
                end
                S_SH_ACT3:
                    st_q <= S_SH_ACT4;
                S_SH_ACT4: begin
                    {a_bad, a_scal, a_prof, a_lvl, a_cf, a_bdy, a_bdc, a_lmfn,
                     a_poct, a_lmpoc, a_dpoaz, a_maxref, a_gaps, a_w, a_h,
                     a_fmo, a_d8, a_crop} <= sps_rd_q;
                    if (!sps_valid_q[ap_sps])
                        fail(E_MISSING, 16'd0);
                    else if (sps_rd_q[SPSW-1])
                        fail(E_SPS, 16'd0);
                    else if (ap_fmo)
                        fail(E_PPS, 16'd0);
                    else if (ap_bad)
                        fail(E_PPS, 16'd0);
                    else if (ap_cabac || ap_scal || sps_rd_q[SPSW-2] ||
                             (stype_q != 3'd0 && stype_q != 3'd2))
                        fail(E_UNSUP, 16'd0);
                    else
                        st_q <= S_SH_FN;
                end
                S_SH_FN: begin
                    fn_q   <= uval[15:0];
                    npoc_q <= 2'd0;
                    if (idr_q) st_q <= S_SH_IDR;
                    else if (a_poct == 2'd0) st_q <= S_SH_POCL;
                    else if (a_poct == 2'd1 && !a_dpoaz) st_q <= S_SH_DP0;
                    else after_poc;
                end
                S_SH_IDR: begin
                    if (a_poct == 2'd0) st_q <= S_SH_POCL;
                    else if (a_poct == 2'd1 && !a_dpoaz) st_q <= S_SH_DP0;
                    else after_poc;
                end
                S_SH_POCL: begin
                    poc0_q <= {2'd0, 6'd0, uval[15:0]};
                    npoc_q <= 2'd1;
                    if (ap_bfpoc) st_q <= S_SH_DPB; else after_poc;
                end
                S_SH_DPB, S_SH_DP0, S_SH_DP1: begin
                    if (!fits(sev, 6'd22))
                        fail(E_SYNTAX, 16'd0);
                    else begin
                        if (npoc_q == 2'd0)
                            poc0_q <= {(st_q == S_SH_DP0) ? 2'd2 : 2'd1, sev[21:0]};
                        else
                            poc1_q <= {(st_q == S_SH_DPB) ? 2'd1 : 2'd3, sev[21:0]};
                        npoc_q <= npoc_q + 2'd1;
                        if (st_q == S_SH_DP0 && ap_bfpoc) st_q <= S_SH_DP1;
                        else after_poc;
                    end
                end
                S_SH_RPC: begin
                    // A redundant slice is dropped whole, silently.
                    if (uev != 32'd0) st_q <= S_DROP;
                    else st_q <= (first_q == 16'd0) ? S_E_DIMS : S_E_HDR;
                end
                S_E_DIMS: begin emit(8'h01, {a_w, a_h}); st_q <= S_E_INFO; end
                S_E_INFO: begin
                    emit(8'h07, {a_prof, a_lvl, a_maxref, a_fmo, a_d8, a_gaps});
                    st_q <= S_E_POCT;
                end
                S_E_POCT: begin
                    emit(8'h08, {a_poct, a_lmfn, (a_poct == 2'd0) ? a_lmpoc : 5'd0, a_cf,
                                 a_bdy, a_bdc, a_dpoaz, 3'd0});
                    st_q <= S_E_CROP;
                end
                S_E_CROP: begin emit(8'h09, a_crop); st_q <= S_E_HDR; end
                S_E_HDR: begin
                    emit(8'h02, {1'b0, stype_q, idr_q, nri_q, ap_cabac, ap_cip, ap_t8,
                                 ap_wp, ap_wb, 3'd0, pps_id_q});
                    st_q <= S_E_FIRST;
                end
                S_E_FIRST: begin
                    emit(8'h06, {8'd0, first_q});
                    if (npoc_q != 2'd0) st_q <= S_E_POC0;
                    else st_q <= is_p ? S_SH_OVR : S_E_REF;
                    l0_q <= ap_l0[3:0];
                end
                S_E_POC0: begin
                    emit(8'h0A, poc0_q);
                    if (npoc_q == 2'd2) st_q <= S_E_POC1;
                    else st_q <= is_p ? S_SH_OVR : S_E_REF;
                end
                S_E_POC1: begin
                    emit(8'h0A, poc1_q);
                    st_q <= is_p ? S_SH_OVR : S_E_REF;
                end
                S_SH_OVR: begin
                    if (uval[0]) st_q <= S_SH_L0;
                    else if (ap_l0 > 5'd15) fail(E_SYNTAX, 16'd0);
                    else st_q <= S_E_REF;
                end
                S_SH_L0: begin
                    l0_q <= uev[3:0];
                    if (uev > 32'd15) fail(E_SYNTAX, 16'd0); else st_q <= S_E_REF;
                end
                S_E_REF: begin
                    emit(8'h05, {is_p ? l0_q : 4'd0, 4'd0, fn_q});
                    if (!is_p) l0_q <= 4'd0;
                    after_ref;
                end
                S_RPLM_F: begin
                    if (uval[0]) st_q <= S_RPLM_IDC; else after_rplm;
                end
                S_RPLM_IDC: begin
                    mmco_op_q <= uev[2:0];
                    if (uev == 32'd3) after_rplm;
                    else if (uev > 32'd3) fail(E_SYNTAX, 16'd0);
                    else st_q <= S_RPLM_V;
                end
                S_RPLM_V: begin
                    if (uev > 32'h1FFFF) fail(E_SYNTAX, 16'd0);
                    else begin
                        emit(8'h0B, {1'b0, mmco_op_q[1:0], 4'd0, uev[16:0]});
                        st_q <= S_RPLM_IDC;
                    end
                end
                S_PWT_LD: begin
                    pwt_w_q <= uev[7:0];
                    if (uev > 32'd7) fail(E_SYNTAX, 16'd0); else st_q <= S_PWT_CD;
                end
                S_PWT_CD: begin
                    if (uev > 32'd7) fail(E_SYNTAX, 16'd0);
                    else begin
                        emit(8'h0C, {18'd0, uev[2:0], pwt_w_q[2:0]});
                        pwt_i_q <= 5'd0;
                        st_q <= S_PWT_LF;
                    end
                end
                S_PWT_LF: begin
                    if (uval[0]) st_q <= S_PWT_LW; else st_q <= S_PWT_CF;
                end
                S_PWT_LW, S_PWT_CW: begin
                    pwt_w_q <= sev[7:0];
                    if (!fits(sev, 6'd8)) fail(E_SYNTAX, 16'd0);
                    else st_q <= (st_q == S_PWT_LW) ? S_PWT_LO : S_PWT_CO;
                end
                S_PWT_LO: begin
                    if (!fits(sev, 6'd8)) fail(E_SYNTAX, 16'd0);
                    else begin
                        emit(8'h0D, {1'b0, pwt_i_q, 2'd0, pwt_w_q, sev[7:0]});
                        st_q <= S_PWT_CF;
                    end
                end
                S_PWT_CF: begin
                    pwt_j_q <= 1'b0;
                    if (uval[0]) st_q <= S_PWT_CW;
                    else if (pwt_i_q[3:0] == l0_q) after_pwt;
                    else begin
                        pwt_i_q <= pwt_i_q + 5'd1;
                        st_q <= S_PWT_LF;
                    end
                end
                S_PWT_CO: begin
                    if (!fits(sev, 6'd8)) fail(E_SYNTAX, 16'd0);
                    else begin
                        emit(8'h0D, {1'b0, pwt_i_q, pwt_j_q ? 2'd2 : 2'd1, pwt_w_q, sev[7:0]});
                        pwt_j_q <= 1'b1;
                        if (!pwt_j_q) st_q <= S_PWT_CW;
                        else if (pwt_i_q[3:0] == l0_q) after_pwt;
                        else begin
                            pwt_i_q <= pwt_i_q + 5'd1;
                            st_q <= S_PWT_LF;
                        end
                    end
                end
                S_DRPM_IDR: begin
                    emit(8'h0E, {21'd0, uval[1:0], 1'b0});
                    st_q <= S_SH_QPD;
                end
                S_DRPM_A: begin
                    emit(8'h0E, {23'd0, uval[0]});
                    st_q <= uval[0] ? S_MMCO_OP : S_SH_QPD;
                end
                S_MMCO_OP: begin
                    mmco_op_q <= uev[2:0];
                    mmco_x_q  <= 16'd0;
                    if (uev == 32'd0) st_q <= S_SH_QPD;
                    else if (uev > 32'd6) fail(E_SYNTAX, 16'd0);
                    else if (uev <= 32'd3) st_q <= S_MMCO_X;
                    else if (uev == 32'd5) emit(8'h0F, {3'd5, 21'd0});
                    else st_q <= S_MMCO_Y;
                end
                S_MMCO_X: begin
                    mmco_x_q <= uev[15:0];
                    if (uev > 32'hFFFF) fail(E_SYNTAX, 16'd0);
                    else if (mmco_op_q == 3'd3) st_q <= S_MMCO_Y;
                    else begin
                        emit(8'h0F, {mmco_op_q, 5'd0, uev[15:0]});
                        st_q <= S_MMCO_OP;
                    end
                end
                S_MMCO_Y: begin
                    if (uev > 32'd31) fail(E_SYNTAX, 16'd0);
                    else begin
                        emit(8'h0F, {mmco_op_q, uev[4:0], mmco_x_q});
                        st_q <= S_MMCO_OP;
                    end
                end
                S_SH_QPD: begin
                    // SliceQPY = 26 + pic_init_qp_minus26 + slice_qp_delta.
                    sqp_q <= ap_qp + sev[5:0];
                    if ($signed(sev) < -$signed({26'd0, ap_qp}) ||
                        $signed(sev) > 51 - $signed({26'd0, ap_qp}))
                        fail(E_SYNTAX, 16'd0);
                    else begin
                        dfi_q <= 2'd0;
                        fa_q  <= 8'd0;
                        fb_q  <= 8'd0;
                        st_q  <= ap_dfc ? S_SH_DFI : S_E_QP;
                    end
                end
                S_SH_DFI: begin
                    dfi_q <= uev[1:0];
                    if (uev > 32'd2) fail(E_SYNTAX, 16'd0);
                    else st_q <= (uev == 32'd1) ? S_E_QP : S_SH_FA;
                end
                S_SH_FA, S_SH_FB: begin
                    // FilterOffsetA/B = slice_*_offset_div2 << 1, -12..12.
                    if ($signed(sev) < -6 || $signed(sev) > 6) fail(E_SYNTAX, 16'd0);
                    else begin
                        if (st_q == S_SH_FA) begin
                            fa_q <= {sev[6:0], 1'b0};
                            st_q <= S_SH_FB;
                        end else begin
                            fb_q <= {sev[6:0], 1'b0};
                            st_q <= S_E_QP;
                        end
                    end
                end
                S_E_QP: begin
                    emit(8'h03, {2'd0, sqp_q, {3{ap_cqp[4]}}, ap_cqp, {3{ap_cqp2[4]}}, ap_cqp2});
                    st_q <= S_E_DBLK;
                end
                S_E_DBLK: begin
                    emit(8'h04, {dfi_q, 6'd0, fa_q, fb_q});
                    qp_q   <= sqp_q;
                    rem_q  <= first_q;
                    mby_q  <= 12'd0;
                    addr_q <= first_q;
                    cnt_q  <= 8'd0;
                    pic_done_q <= 1'b0;
                    st_q   <= S_SD_DIV;
                end
                S_SD_DIV: begin
                    // first_mb_in_slice to a position, by subtraction: at
                    // most FrameHeightInMbs cycles a slice.
                    if (rem_q >= {4'd0, a_w}) begin
                        rem_q <= rem_q - {4'd0, a_w};
                        mby_q <= mby_q + 12'd1;
                    end else begin
                        mbx_q <= rem_q[6:0];
                        st_q  <= S_SD_LOOP;
                    end
                end

                // ---------------------------------------------------------
                // Slice data, 7.3.4
                S_SD_LOOP:
                    st_q <= is_p ? S_SKIPRUN : S_MB_START;
                S_SKIPRUN: begin
                    skip_q <= uev[15:0];
                    if (uev > 32'hFFFF) fail(E_SYNTAX, addr_q);
                    else st_q <= (uev == 32'd0) ? S_MB_START : S_SK_START;
                end
                S_SK_START: begin
                    if (mby_q >= a_h) fail(E_SYNTAX, addr_q);
                    else begin
                        emit(8'h10, {5'd0, mbx_q, mby_q});
                        in_skip_q <= 1'b1;
                        kind_q <= K_PSKIP;
                        cbp_q  <= 6'd0;
                        t8_q   <= 1'b0;
                        clear_mb(5'd0);
                        st_q <= S_SK_TYPE;
                    end
                end
                S_SK_TYPE: begin emit(8'h11, {K_PSKIP, 19'd0}); st_q <= S_SK_CBP; end
                S_SK_CBP:  begin emit(8'h19, 24'd0); st_q <= S_SK_QP; end
                S_SK_QP:   begin emit(8'h12, {16'd0, 2'd0, qp_q}); st_q <= S_MB_END; end
                S_MB_END: begin
                    emit(8'h3F, 24'd0);
                    ltcy_v[0 +: 5] <= tcy_v[15 +: 5];  ltcy_v[5 +: 5] <= tcy_v[35 +: 5];
                    ltcy_v[10 +: 5] <= tcy_v[55 +: 5]; ltcy_v[15 +: 5] <= tcy_v[75 +: 5];
                    lmd_v[0 +: 4]  <= md_v[12 +: 4];   lmd_v[4 +: 4]  <= md_v[28 +: 4];
                    lmd_v[8 +: 4]  <= md_v[44 +: 4];  lmd_v[12 +: 4]  <= md_v[60 +: 4];
                    ltcb_v[0 +: 5] <= tcb_v[5 +: 5];  ltcb_v[5 +: 5] <= tcb_v[15 +: 5];
                    ltcr_v[0 +: 5] <= tcr_v[5 +: 5];  ltcr_v[5 +: 5] <= tcr_v[15 +: 5];
                    linter_q <= (kind_q >= K_P16);
                    if (last_mb) pic_done_q <= 1'b1;
                    addr_q <= addr_q + 16'd1;
                    if (cnt_q != 8'hFF) cnt_q <= cnt_q + 8'd1;
                    if ({5'd0, mbx_q} == a_w - 12'd1) begin
                        mbx_q <= 7'd0;
                        mby_q <= mby_q + 12'd1;
                    end else
                        mbx_q <= mbx_q + 7'd1;
                    if (in_skip_q) begin
                        skip_q <= skip_q - 16'd1;
                        if (skip_q == 16'd1) begin
                            in_skip_q <= 1'b0;
                            // After a run: more_rbsp_data(), then a
                            // macroblock_layer() if there is more.
                            ret_q <= S_MB_START;
                            st_q  <= S_AFTER;
                        end else
                            st_q <= S_SK_START;
                    end else begin
                        ret_q <= S_SD_LOOP;
                        st_q  <= S_AFTER;
                    end
                end
                S_AFTER:
                    st_q <= more ? ret_q : S_SLICE_END;
                S_SLICE_END: begin
                    emit(8'h40, 24'd0);
                    st_q <= pic_done_q ? S_PIC_END : S_DROP;
                end
                S_PIC_END: begin
                    emit(8'h41, 24'd0);
                    st_q <= S_DROP;
                end

                // ---------------------------------------------------------
                // macroblock_layer(), 7.3.5
                S_MB_START: begin
                    if (mby_q >= a_h) fail(E_SYNTAX, addr_q);
                    else begin
                        emit(8'h10, {5'd0, mbx_q, mby_q});
                        t8_q  <= 1'b0;
                        cpm_q <= 2'd0;
                        clear_mb(5'd0);
                        st_q <= S_MB_TYPE;
                    end
                end
                S_MB_TYPE: begin
                    if (!mt_ok) fail(E_SYNTAX, addr_q);
                    else begin
                        kind_q <= mt_kind;
                        i16m_q <= mt_i16m;
                        cbp_q  <= mt_cbp;
                        part_q <= 2'd0;
                        subp_q <= 2'd0;
                        comp_q <= 1'b0;
                        ip_q   <= 5'd0;
                        emit(8'h11, {mt_kind, (mt_kind == K_P16 || mt_kind == K_P168 || mt_kind == K_P816) ? 2'd1 : 2'd0,
                                     (mt_kind == K_P168 || mt_kind == K_P816) ? 2'd1 : 2'd0,
                                     mt_i16m, 13'd0});
                        case (mt_kind)
                            K_PCM:       st_q <= S_PCM_CBP;
                            K_P8, K_P8R0: st_q <= S_SUB;
                            K_INXN:      st_q <= ap_t8 ? S_T8A : S_IPRED;
                            K_I16:       st_q <= S_CPM;
                            default:     st_q <= S_REF;
                        endcase
                    end
                end
                S_PCM_CBP: begin emit(8'h19, 24'd0); st_q <= S_PCM_QP; end
                S_PCM_QP: begin
                    emit(8'h12, {18'd0, qp_q});
                    pcm_q <= 7'd0;
                    st_q <= S_PCM_ALIGN;
                end
                S_PCM_ALIGN:
                    st_q <= S_PCM_DATA;
                S_PCM_DATA: begin
                    emit(8'h30, {uval[7:0], uval[15:8], uval[23:16]});
                    pcm_q <= pcm_q + 7'd1;
                    if (pcm_q == 7'd127) begin
                        clear_mb(5'd16);
                        st_q <= S_MB_END;
                    end
                end
                S_SUB: begin
                    if (uev > 32'd3) fail(E_SYNTAX, addr_q);
                    else begin
                        sub_q <= put2x4(sub_q, part_q, uev[1:0]);
                        part_q <= part_q + 2'd1;
                        if (part_q == 2'd3) st_q <= S_SUB_TOK;
                    end
                end
                S_SUB_TOK: begin
                    emit(8'h15, {8'd0, 2'd0, sub_q[1:0], 2'd0, sub_q[3:2],
                                 2'd0, sub_q[5:4], 2'd0, sub_q[7:6]});
                    part_q <= 2'd0;
                    st_q <= S_REF;
                end
                S_T8A: begin t8_q <= uval[0]; st_q <= S_IPRED; end
                S_IPRED: begin
                    if (t8_q) begin
                        emit(8'h17, {18'd0, ip_q[1:0], mfinal});
                        // The 8x8 block's four 4x4 positions, so that a
                        // later Intra_4x4 or Intra_8x8 neighbour finds it.
                        md_v <= put4x16(put4x16(put4x16(put4x16(md_v,
                                    {ly, lx}, mfinal), {ly, lx + 2'd1}, mfinal),
                                    {ly + 2'd1, lx}, mfinal), {ly + 2'd1, lx + 2'd1}, mfinal);
                    end else begin
                        emit(8'h13, {16'd0, ip_q[3:0], mfinal});
                        md_v <= put4x16(md_v, lr, mfinal);
                    end
                    ip_q <= ip_q + 5'd1;
                    if (ip_q == (t8_q ? 5'd3 : 5'd15)) st_q <= S_CPM;
                end
                S_CPM: begin
                    cpm_q <= uev[1:0];
                    if (uev > 32'd3) fail(E_SYNTAX, addr_q);
                    else st_q <= (kind_q == K_I16) ? S_CBPTOK : S_CBP;
                end
                S_REF: begin
                    // ref_idx_l0, te(v) with range num_ref_idx_l0_active_minus1;
                    // emitted even when inferred to be 0.
                    if (rt == RT_UE && uev > {28'd0, l0_q}) fail(E_SYNTAX, addr_q);
                    else begin
                        emit(8'h18, {1'b0, part_q, 16'd0,
                                     (rt == RT_UE) ? uev[4:0] : (rt == RT_U) ? {4'd0, !uval[0]} : 5'd0});
                        part_q <= part_q + 2'd1;
                        if (part_q == nparts_m1) begin
                            part_q <= 2'd0;
                            subp_q <= 2'd0;
                            comp_q <= 1'b0;
                            st_q   <= S_MVD;
                        end
                    end
                end
                S_MVD: begin
                    if (!fits(sev, 6'd16)) fail(E_SYNTAX, addr_q);
                    else begin
                        emit(8'h14, {1'b0, part_q, subp_q, comp_q, 2'd0, sev[15:0]});
                        comp_q <= !comp_q;
                        if (comp_q) begin
                            if (subp_q == nsub_m1) begin
                                subp_q <= 2'd0;
                                part_q <= part_q + 2'd1;
                                if (part_q == nparts_m1) st_q <= S_CBP;
                            end else
                                subp_q <= subp_q + 2'd1;
                        end
                    end
                end
                S_CBP: begin
                    cbp_q <= cbp_val;
                    if (!cbp_ok) fail(E_SYNTAX, addr_q);
                    else if (cbp_val[3:0] != 4'd0 && ap_t8 && kind_q != K_INXN &&
                             !((kind_q == K_P8 || kind_q == K_P8R0) && sub_q != 8'd0))
                        st_q <= S_T8B;
                    else
                        st_q <= S_CBPTOK;
                end
                S_T8B: begin t8_q <= uval[0]; st_q <= S_CBPTOK; end
                S_CBPTOK: begin
                    emit(8'h19, {15'd0, t8_q, cpm_q, cbp_q});
                    st_q <= (cbp_q != 6'd0 || kind_q == K_I16) ? S_QPD : S_QPTOK;
                end
                S_QPD: begin
                    if ($signed(sev) < -26 || $signed(sev) > 25) fail(E_SYNTAX, addr_q);
                    else begin
                        qp_q <= qnew;
                        emit(8'h12, {18'd0, qnew});
                        res_q <= 5'd0;
                        st_q <= S_RES;
                    end
                end
                S_QPTOK: begin
                    emit(8'h12, {18'd0, qp_q});
                    st_q <= S_MB_END;
                end
                S_RES: begin
                    if (res_q == 5'd27)
                        st_q <= S_MB_END;
                    else if (r_coded) begin
                        // The nC walk is long: latch its answer, and start
                        // the block from registers a cycle later.
                        rq_nc   <= r_nc;
                        rq_dc   <= (r_cls == 3'd4);
                        rq_kind <= r_kind;
                        rq_cat  <= r_cat;
                        rq_blk  <= r_blk;
                        st_q    <= S_RES2;
                    end else
                        res_q <= res_q + 5'd1;
                end
                S_RES2: begin
                    st_q   <= S_RESW;
                    resw_q <= 1'b1;
                end
                S_RESW: begin
                    if (cav_tok_we && !cav_err)
                        emit(cav_tok[31:24], cav_tok[23:0]);
                    if (cav_err)
                        fail(E_SYNTAX, addr_q);
                    else if (cav_done) begin
                        if (res_q >= 5'd1 && res_q <= 5'd16)
                            tcy_v <= put5x16(tcy_v, rr, cav_tc);
                        else if (res_q >= 5'd19 && res_q <= 5'd22)
                            tcb_v <= put5x4(tcb_v, cidx, cav_tc);
                        else if (res_q >= 5'd23)
                            tcr_v <= put5x4(tcr_v, cidx, cav_tc);
                        res_q  <= res_q + 5'd1;
                        st_q   <= S_RES;
                        resw_q <= 1'b0;
                    end
                end
                default:
                    st_q <= S_DROP;
                endcase
            end
        end
    end
endmodule
