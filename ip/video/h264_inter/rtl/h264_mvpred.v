// h264_mvpred — the motion vectors of ITU-T H.264 clause 8.4.1, in the
// token stream between `h264_parse` and `h264_recon`.
//
// What it does
//   Every token passes through unchanged and in order, except `MVD`,
//   which is consumed. In its place each inter macroblock gets its final
//   motion vectors: for each 4x4 block, in luma4x4BlkIdx order, two `MV`
//   tokens (horizontal, then vertical), inserted **immediately before
//   the macroblock's `MB_CBP`**. A P_Skip macroblock gets them too, at the
//   same place, so a consumer never has to know which vectors were coded
//   and which were derived. `docs/h264-decoder.md` has the token format.
//
//   `MV` (tag 0x16):
//     [23]     list (0 for now; 1 is reserved for B slices)
//     [22:19]  luma4x4BlkIdx
//     [18]     component: 0 horizontal, 1 vertical
//     horizontal: [15:0]  mvLX[0], signed, quarter samples
//     vertical:   [17:13] refIdxLX, [12:0] mvLX[1], signed
//   The vertical component is 13 bits because Table A-1 bounds it to
//   [-512, +511.75] samples at every level (12 bits are enough); the
//   horizontal one can reach [-2048, +2047.75] and needs 14.
//
// The derivation, clause by clause
//   * 8.4.1.3.2 / 6.4.11.7: the neighbours A, B, C of a partition are the
//     4x4 blocks left of, above, and above-right of its top-left corner
//     (C at x + predPartWidth); C is replaced by D, above-left, when C is
//     not available. A neighbour inside the current macroblock is
//     available only once its partition has been decoded, which a mask
//     of decoded 4x4 blocks answers exactly (NOTE 2 of 6.4.11.7 is that
//     case). One that is unavailable, or intra, gives mv 0 and refIdx -1.
//   * 6.4.9 / 6.4.8: a neighbouring macroblock is available when it is
//     inside the picture and in the current slice. Slices are contiguous
//     in raster order here (no ASO, no FMO), so "in the slice" is "at or
//     after the slice's first macroblock", whose position is taken from
//     the first `MB_START` after `SLICE_FIRST`.
//   * 8.4.1.3: the 16x8 and 8x16 directional rules, else the median of
//     8.4.1.3.1, with its rule that B and C both unavailable and A
//     available make B and C copies of A, and its rule that exactly one
//     matching refIdx picks that neighbour.
//   * 8.4.1.1: P_Skip is refIdx 0 and a zero vector when A or B is
//     unavailable or either is refIdx 0 with a zero vector; otherwise
//     the 16x16 prediction.
//
// Storage
//   The current macroblock's sixteen blocks, the left macroblock's right
//   column and six blocks of the row above (above-left, the four above,
//   above-right) are registers. The bottom row of every macroblock goes
//   into a block RAM line buffer, four entries a macroblock column, so a
//   1280-wide picture is 320 entries of 38 bits. The above-left block of
//   a macroblock is not read from the line buffer, because the previous
//   macroblock's bottom row has already overwritten it there: it is the
//   previous macroblock's above-right-most block of the row above,
//   saved before that write.
//
// What it does not do (yet)
//   List 1. `MVD` and `REF_IDX` tokens of list 1 are consumed and
//   ignored and B macroblocks get no vectors; the token format, the
//   `list` bit and the `kind` decode leave room for B_8x8, direct and
//   the second line buffer that will need. MBAFF and field pictures are
//   out (frame_mbs_only_flag = 1 is what the parser accepts).
module h264_mvpred #(
    // The widest picture, in macroblocks: 80 is 1280 samples.
    parameter integer MAX_WIDTH_MBS = 80
) (
    input  wire        clk,
    input  wire        rst_n,

    input  wire [31:0] in_tok,
    input  wire        in_valid,
    output wire        in_ready,

    output wire [31:0] out_tok,
    output wire        out_valid,
    input  wire        out_ready
);
    localparam integer LB_DEPTH = 4 * MAX_WIDTH_MBS;
    localparam integer LB_AW    = $clog2(LB_DEPTH);
    localparam integer MBX_W    = $clog2(MAX_WIDTH_MBS + 1);

    localparam [7:0] T_SPS_DIMS    = 8'h01;
    localparam [7:0] T_SLICE_FIRST = 8'h06;
    localparam [7:0] T_MB_START    = 8'h10;
    localparam [7:0] T_MB_TYPE     = 8'h11;
    localparam [7:0] T_MVD         = 8'h14;
    localparam [7:0] T_SUB_MB_TYPE = 8'h15;
    localparam [7:0] T_MV          = 8'h16;
    localparam [7:0] T_REF_IDX     = 8'h18;
    localparam [7:0] T_MB_CBP      = 8'h19;
    localparam [7:0] T_MB_END      = 8'h3F;

    localparam [2:0] S_PASS   = 3'd0;  // tokens flow
    localparam [2:0] S_LOAD   = 3'd1;  // read the row above
    localparam [2:0] S_PRED   = 3'd2;  // one partition's vector
    localparam [2:0] S_EMIT   = 3'd3;  // the MV tokens, then the held MB_CBP
    localparam [2:0] S_COMMIT = 3'd4;  // the bottom row into the line buffer

    reg [2:0] state;

    // A block's motion: {refIdx (signed, -1 = none), mvy, mvx}.
    localparam integer MW = 38;

    // ---------------------------------------------------------------
    // Picture, slice and macroblock position
    // ---------------------------------------------------------------
    reg [MBX_W-1:0] width_mbs;
    reg [11:0]      mb_x;
    reg [11:0]      mb_y;
    reg [11:0]      first_x;
    reg [11:0]      first_y;
    reg             need_first;

    reg [4:0]       kind;
    reg [7:0]       sub_types;      // two bits each, Table 7-17's values
    reg [5:0]       ref0 [0:3];     // refIdxL0 per mbPartIdx
    reg             inter_mb;       // this macroblock gets MV tokens
    reg             emitted;        // and has had them

    // Availability of the neighbouring macroblocks, 6.4.9 with the slice
    // rule. "Row above is in the slice" is (mb_y - 1, x) >= (first_y,
    // first_x) in raster order.
    wire        above_row_after = (mb_y > first_y + 12'd1);
    wire        above_row_same  = (mb_y == first_y + 12'd1);
    wire        avail_a = (mb_x != 12'd0) &&
                          ((mb_y != first_y) || (mb_x > first_x));
    wire        avail_b = (mb_y != 12'd0) &&
                          (above_row_after || (above_row_same && mb_x >= first_x));
    wire        avail_c = (mb_y != 12'd0) &&
                          (mb_x + 12'd1 < {{(12-MBX_W){1'b0}}, width_mbs}) &&
                          (above_row_after || (above_row_same && mb_x + 12'd1 >= first_x));
    wire        avail_d = (mb_y != 12'd0) && (mb_x != 12'd0) &&
                          (above_row_after || (above_row_same && mb_x >= first_x + 12'd1));

    // ---------------------------------------------------------------
    // Motion storage
    // ---------------------------------------------------------------
    // The current macroblock, indexed in raster order of 4x4 blocks
    // (4 * row + column), which is what the neighbour arithmetic wants;
    // luma4x4BlkIdx is only the order the tokens leave in. A distributed
    // RAM written one block a cycle: every block of an inter macroblock is
    // written exactly once, so that is sixteen cycles a macroblock, and an
    // intra macroblock is not written at all -- its commit writes "none"
    // into the line buffer and the left column instead. Nothing reads a
    // block before it is written, because `decoded` gates every read.
    reg [MW-1:0] cur   [0:15];
    reg          cur_we;
    reg [3:0]    cur_wa;
    reg [MW-1:0] cur_wd;
    always @(posedge clk) begin
        if (cur_we) cur[cur_wa] <= cur_wd;
    end
    reg [15:0]   decoded;
    reg [MW-1:0] left  [0:3];
    reg [MW-1:0] above [0:5];       // [0] above-left, [1..4] above, [5] above-right
    reg [MW-1:0] prev_above3;       // the previous macroblock's above[4]

    localparam [MW-1:0] NONE = {6'h3F, 16'd0, 16'd0};

    reg [MW-1:0] lb [0:LB_DEPTH-1];
    reg [MW-1:0] lb_q;
    reg [LB_AW-1:0] lb_raddr;
    reg [LB_AW-1:0] lb_waddr;
    reg [MW-1:0] lb_wdata;
    reg          lb_we;

    always @(posedge clk) begin
        if (lb_we) lb[lb_waddr] <= lb_wdata;
        lb_q <= lb[lb_raddr];
    end

    // ---------------------------------------------------------------
    // Neighbour lookup, in units of 4x4 blocks
    // ---------------------------------------------------------------
    // (xn, yn) is relative to the macroblock, -1..4 and -1..3, offset by
    // one so that both are unsigned: xo = xn + 1, yo = yn + 1.
    function [MW:0] lookup;        // {available, motion}
        input [2:0] xo;
        input [2:0] yo;
        begin
            if (yo == 3'd0) begin
                if (xo == 3'd0)      lookup = {avail_d, avail_d ? above[0] : NONE};
                else if (xo == 3'd5) lookup = {avail_c, avail_c ? above[5] : NONE};
                else                 lookup = {avail_b, avail_b ? above[xo] : NONE};
            end else if (xo == 3'd0) begin
                lookup = {avail_a, avail_a ? left[yo - 3'd1] : NONE};
            end else if (xo == 3'd5) begin
                lookup = {1'b0, NONE};
            end else if (decoded[{yo[1:0] - 2'd1, xo[1:0] - 2'd1}]) begin
                lookup = {1'b1, cur[{yo[1:0] - 2'd1, xo[1:0] - 2'd1}]};
            end else begin
                lookup = {1'b0, NONE};
            end
        end
    endfunction

    function signed [15:0] median3;
        input signed [15:0] a;
        input signed [15:0] b;
        input signed [15:0] c;
        reg   signed [15:0] lo;
        reg   signed [15:0] hi;
        begin
            lo = (a < b) ? a : b;
            hi = (a < b) ? b : a;
            median3 = (c < lo) ? lo : ((c > hi) ? hi : c);
        end
    endfunction

    // ---------------------------------------------------------------
    // The partition being predicted
    // ---------------------------------------------------------------
    reg [1:0]  p_part;              // mbPartIdx
    reg [1:0]  p_sub;               // subMbPartIdx
    reg        p_skip;
    reg signed [15:0] mvd_x;
    reg signed [15:0] mvd_y;
    reg [3:0]  p_step;

    // Geometry of (p_part, p_sub) in 4x4 units: corner, width, height.
    reg [1:0]  gx, gy;
    reg [2:0]  gw, gh;
    reg  [1:0] p_subtype;
    always @* begin
        case (p_part)
            2'd0: p_subtype = sub_types[7:6];
            2'd1: p_subtype = sub_types[5:4];
            2'd2: p_subtype = sub_types[3:2];
            default: p_subtype = sub_types[1:0];
        endcase
    end
    always @* begin
        gx = 2'd0; gy = 2'd0; gw = 3'd4; gh = 3'd4;
        case (kind)
            5'd5: begin gy = {p_part[0], 1'b0}; gh = 3'd2; end           // 16x8
            5'd6: begin gx = {p_part[0], 1'b0}; gw = 3'd2; end           // 8x16
            5'd7, 5'd8: begin                                            // P_8x8
                gx = {p_part[0], 1'b0};
                gy = {p_part[1], 1'b0};
                case (p_subtype)
                    2'd0: begin gw = 3'd2; gh = 3'd2; end
                    2'd1: begin gw = 3'd2; gh = 3'd1; gy = {p_part[1], p_sub[0]}; end
                    2'd2: begin gw = 3'd1; gh = 3'd2; gx = {p_part[0], p_sub[0]}; end
                    default: begin
                        gw = 3'd1; gh = 3'd1;
                        gx = {p_part[0], p_sub[0]};
                        gy = {p_part[1], p_sub[1]};
                    end
                endcase
            end
            default: ;                                                   // 16x16, P_Skip
        endcase
    end

    wire [5:0]  p_ref = p_skip ? 6'd0 : ref0[p_part];

    // Steps 0..3: the neighbours A, B, C, D, one lookup a cycle through
    // one multiplexer, registered. Step 4 replaces C by D when C is not
    // available.
    reg  [2:0] q_xo, q_yo;
    always @* begin
        case (p_step[1:0])
            2'd0:    begin q_xo = {1'b0, gx};             q_yo = {1'b0, gy} + 3'd1; end  // A
            2'd1:    begin q_xo = {1'b0, gx} + 3'd1;      q_yo = {1'b0, gy};        end  // B
            2'd2:    begin q_xo = {1'b0, gx} + gw + 3'd1; q_yo = {1'b0, gy};        end  // C
            default: begin q_xo = {1'b0, gx};             q_yo = {1'b0, gy};        end  // D
        endcase
    end
    wire [MW:0] nb_q = lookup(q_xo, q_yo);

    reg [MW:0] na, nb, nc, nd;

    // Step 5: which neighbour predicts, 8.4.1.3, registered with the
    // median's inputs after 8.4.1.3.1's first rule. Step 6: the predictor.
    wire signed [5:0]  ra = na[37:32];
    wire signed [5:0]  rb = nb[37:32];
    wire signed [5:0]  rc = nc[37:32];
    wire               bc_gone = !nb[MW] && !nc[MW] && na[MW];
    wire signed [5:0]  rb2 = bc_gone ? ra : rb;
    wire signed [5:0]  rc2 = bc_gone ? ra : rc;
    wire               ma  = (ra  == p_ref);
    wire               mbm = (rb2 == p_ref);
    wire               mcm = (rc2 == p_ref);
    localparam [2:0] PK_MED = 3'd0, PK_A = 3'd1, PK_B = 3'd2, PK_C = 3'd3,
                     PK_B2 = 3'd4, PK_C2 = 3'd5;
    reg  [2:0]         pick;
    always @* begin
        if (kind == 5'd5 && p_part == 2'd0 && rb == p_ref)       pick = PK_B;
        else if (kind == 5'd5 && p_part == 2'd1 && ra == p_ref)  pick = PK_A;
        else if (kind == 5'd6 && p_part == 2'd0 && ra == p_ref)  pick = PK_A;
        else if (kind == 5'd6 && p_part == 2'd1 && rc == p_ref)  pick = PK_C;
        else if (ma && !mbm && !mcm)  pick = PK_A;
        else if (!ma && mbm && !mcm)  pick = PK_B2;
        else if (!ma && !mbm && mcm)  pick = PK_C2;
        else pick = PK_MED;
    end
    reg  [2:0]         pick_q;
    reg  [31:0]        mb2q, mc2q;  // B and C after the first median rule
    wire [15:0]        med_y = median3(na[31:16], mb2q[31:16], mc2q[31:16]);
    wire [15:0]        med_x = median3(na[15:0],  mb2q[15:0],  mc2q[15:0]);
    reg  [31:0]        mvp;         // {y, x}
    always @* begin
        case (pick_q)
            PK_A:    mvp = na[31:0];
            PK_B:    mvp = nb[31:0];
            PK_C:    mvp = nc[31:0];
            PK_B2:   mvp = mb2q;
            PK_C2:   mvp = mc2q;
            default: mvp = {med_y, med_x};
        endcase
    end
    reg  [31:0]        mvp_q;

    // P_Skip's zero rule, 8.4.1.1.
    wire skip_zero = !na[MW] || !nb[MW] ||
                     (ra == 6'd0 && na[31:0] == 32'd0) ||
                     (rb == 6'd0 && nb[31:0] == 32'd0);

    reg [31:0] mv_final;
    reg [1:0]  w_r, w_c;            // the block being written

    always @* begin
        cur_we = (state == S_PRED) && (p_step == 4'd8);
        cur_wa = {gy + w_r, gx + w_c};
        cur_wd = {p_ref, mv_final};
    end

    // ---------------------------------------------------------------
    // Emission of the MV tokens
    // ---------------------------------------------------------------
    reg [5:0]  e_cnt;               // {done, block, component}
    reg [31:0] held;                // the MB_CBP that waits behind them
    // luma4x4BlkIdx to raster: bits {b3, b1} are the row, {b2, b0} the column.
    wire [3:0] e_blk = e_cnt[4:1];
    wire [3:0] e_raster = {e_blk[3], e_blk[1], e_blk[2], e_blk[0]};
    wire [MW-1:0] e_mv = cur[e_raster];
    wire [31:0] e_tok = e_cnt[0]
        ? {T_MV, 1'b0, e_blk, 1'b1, e_mv[36:32], e_mv[28:16]}
        : {T_MV, 1'b0, e_blk, 1'b0, 2'b00, e_mv[15:0]};

    // ---------------------------------------------------------------
    // The stream
    // ---------------------------------------------------------------
    // The output is a two-entry queue, so that `in_ready` is a function
    // of registers only: a stage behind this one cannot make a
    // combinational path through it to the stage in front.
    reg  [31:0] oq0, oq1;
    reg  [1:0]  oq_n;
    assign      out_tok   = oq0;
    assign      out_valid = (oq_n != 2'd0);
    wire        o_pop     = out_valid && out_ready;
    wire        out_free  = (oq_n != 2'd2);
    reg         o_push;
    reg  [31:0] o_d;
    always @(posedge clk) begin
        if (!rst_n) begin
            oq0 <= 32'd0; oq1 <= 32'd0; oq_n <= 2'd0;
        end else begin
            case ({o_push, o_pop})
                2'b10: begin
                    if (oq_n == 2'd0) oq0 <= o_d;
                    else              oq1 <= o_d;
                    oq_n <= oq_n + 2'd1;
                end
                2'b01: begin
                    oq0  <= oq1;
                    oq_n <= oq_n - 2'd1;
                end
                2'b11: begin
                    if (oq_n == 2'd1) oq0 <= o_d;
                    else begin oq0 <= oq1; oq1 <= o_d; end
                end
                default: ;
            endcase
        end
    end

    assign     in_ready = (state == S_PASS) && out_free;
    wire       take     = in_valid && in_ready;
    wire [7:0] tag      = in_tok[31:24];
    wire       passes   = !(tag == T_MVD) && !(tag == T_MB_CBP && inter_mb && !emitted);
    always @* begin
        o_push = 1'b0;
        o_d    = in_tok;
        if (state == S_PASS) begin
            o_push = take && passes;
        end else if (state == S_EMIT) begin
            o_push = out_free;
            o_d    = e_cnt[5] ? held : e_tok;
        end
    end

    reg [2:0]  ld_ix;               // line-buffer reads issued
    reg [2:0]  cm_ix;               // line-buffer writes done
    // The bottom row (12..15) for the line buffer, then the right column
    // (3, 7, 11, 15) for the next macroblock's left; "none" for a
    // macroblock that is not inter.
    wire [3:0]    cm_ra = cm_ix[2] ? {cm_ix[1:0], 2'd3} : {2'd3, cm_ix[1:0]};
    wire [MW-1:0] cm_rd = cur[cm_ra];
    wire [MW-1:0] cm_mv = (inter_mb && decoded == 16'hFFFF) ? cm_rd : NONE;
    integer    k;

    always @(posedge clk) begin
        if (!rst_n) begin
            state       <= S_PASS;
            width_mbs   <= {MBX_W{1'b0}};
            mb_x        <= 12'd0;
            mb_y        <= 12'd0;
            first_x     <= 12'd0;
            first_y     <= 12'd0;
            need_first  <= 1'b0;
            kind        <= 5'd0;
            sub_types   <= 8'd0;
            inter_mb    <= 1'b0;
            emitted     <= 1'b0;
            decoded     <= 16'd0;
            prev_above3 <= NONE;
            p_part      <= 2'd0;
            p_sub       <= 2'd0;
            p_skip      <= 1'b0;
            p_step      <= 4'd0;
            w_r         <= 2'd0;
            w_c         <= 2'd0;
            mvd_x       <= 16'sd0;
            mvd_y       <= 16'sd0;
            na          <= {1'b0, NONE};
            nb          <= {1'b0, NONE};
            nc          <= {1'b0, NONE};
            nd          <= {1'b0, NONE};
            mv_final    <= 32'd0;
            mvp_q       <= 32'd0;
            mb2q        <= 32'd0;
            mc2q        <= 32'd0;
            pick_q      <= 3'd0;
            e_cnt       <= 6'd0;
            held        <= 32'd0;
            ld_ix       <= 3'd0;
            cm_ix       <= 3'd0;
            lb_we       <= 1'b0;
            lb_waddr    <= {LB_AW{1'b0}};
            lb_raddr    <= {LB_AW{1'b0}};
            lb_wdata    <= NONE;
            for (k = 0; k < 4; k = k + 1) begin
                left[k] <= NONE;
                ref0[k] <= 6'd0;
            end
            for (k = 0; k < 6; k = k + 1) above[k] <= NONE;
        end else begin
            lb_we <= 1'b0;

            case (state)
            S_PASS: if (take) begin
                // Everything but MVD goes straight out; MB_CBP of an inter
                // macroblock waits behind the vectors.
                if (tag == T_MVD) begin
                    if (!in_tok[23]) begin
                        p_part <= in_tok[22:21];
                        p_sub  <= in_tok[20:19];
                        p_skip <= 1'b0;
                        if (!in_tok[18]) begin
                            mvd_x <= in_tok[15:0];
                        end else begin
                            mvd_y  <= in_tok[15:0];
                            p_step <= 4'd0;
                            state  <= S_PRED;
                        end
                    end
                end else if (tag == T_MB_CBP && inter_mb && !emitted) begin
                    held  <= in_tok;
                    e_cnt <= 6'd0;
                    state <= S_EMIT;
                end

                case (tag)
                    T_SPS_DIMS:    width_mbs <= in_tok[12+MBX_W-1:12];
                    T_SLICE_FIRST: need_first <= 1'b1;
                    T_MB_START: begin
                        mb_x <= in_tok[23:12];
                        mb_y <= in_tok[11:0];
                        if (need_first) begin
                            first_x    <= in_tok[23:12];
                            first_y    <= in_tok[11:0];
                            need_first <= 1'b0;
                        end
                        ld_ix <= 3'd0;
                        state <= S_LOAD;
                    end
                    T_MB_TYPE: begin
                        kind     <= in_tok[23:19];
                        decoded  <= 16'd0;
                        emitted  <= 1'b0;
                        inter_mb <= (in_tok[23:19] >= 5'd4) && (in_tok[23:19] <= 5'd9);
                        if (in_tok[23:19] == 5'd9) begin
                            p_part <= 2'd0;
                            p_sub  <= 2'd0;
                            p_skip <= 1'b1;
                            mvd_x  <= 16'sd0;
                            mvd_y  <= 16'sd0;
                            p_step <= 4'd0;
                            state  <= S_PRED;
                        end
                    end
                    T_SUB_MB_TYPE: sub_types <= {in_tok[13:12], in_tok[9:8],
                                                 in_tok[5:4], in_tok[1:0]};
                    T_REF_IDX: if (!in_tok[23]) ref0[in_tok[22:21]] <= {1'b0, in_tok[4:0]};
                    T_MB_END: begin
                        cm_ix <= 3'd0;
                        state <= S_COMMIT;
                    end
                    default: ;
                endcase
            end

            // Six reads: above-left comes from the previous macroblock,
            // then entries 4x .. 4x+4 of the line buffer, each landing a
            // cycle after its address.
            S_LOAD: begin
                ld_ix    <= ld_ix + 3'd1;
                if (ld_ix <= 3'd4) lb_raddr <= mb_x[LB_AW-3:0] * 4 + ld_ix;
                if (ld_ix == 3'd0) above[0] <= prev_above3;
                if (ld_ix >= 3'd2) above[ld_ix - 3'd1] <= lb_q;
                if (ld_ix == 3'd6) state <= S_PASS;
            end

            S_PRED: begin
                p_step <= p_step + 4'd1;
                case (p_step)
                    4'd0: na <= nb_q;
                    4'd1: nb <= nb_q;
                    4'd2: nc <= nb_q;
                    4'd3: nd <= nb_q;
                    4'd4: if (!nc[MW]) nc <= nd;
                    4'd5: begin
                        pick_q <= pick;
                        mb2q   <= bc_gone ? na[31:0] : nb[31:0];
                        mc2q   <= bc_gone ? na[31:0] : nc[31:0];
                    end
                    4'd6: mvp_q <= mvp;
                    4'd7: begin
                        if (p_skip && skip_zero) mv_final <= 32'd0;
                        else mv_final <= {mvp_q[31:16] + mvd_y, mvp_q[15:0] + mvd_x};
                    end
                    default: begin
                        // One block a cycle, row by row.
                        decoded[{gy + w_r, gx + w_c}] <= 1'b1;
                        if (w_c + 2'd1 == gw[1:0] || gw == 3'd4 && w_c == 2'd3) begin
                            w_c <= 2'd0;
                            if (w_r + 2'd1 == gh[1:0] || gh == 3'd4 && w_r == 2'd3) begin
                                w_r   <= 2'd0;
                                state <= S_PASS;
                            end else begin
                                w_r <= w_r + 2'd1;
                            end
                        end else begin
                            w_c <= w_c + 2'd1;
                        end
                        p_step <= p_step;
                    end
                endcase
            end

            // Thirty-two vectors, then the MB_CBP they were held for.
            S_EMIT: if (out_free) begin
                if (e_cnt[5]) begin
                    emitted <= 1'b1;
                    state   <= S_PASS;
                end else begin
                    e_cnt   <= e_cnt + 6'd1;
                end
            end

            // The bottom row goes into the line buffer, the right column
            // becomes the next macroblock's left, and above[4] is kept as
            // its above-left.
            S_COMMIT: begin
                cm_ix <= cm_ix + 3'd1;
                if (!cm_ix[2]) begin
                    lb_we    <= 1'b1;
                    lb_waddr <= mb_x[LB_AW-3:0] * 4 + cm_ix[1:0];
                    lb_wdata <= cm_mv;
                end else begin
                    left[cm_ix[1:0]] <= cm_mv;
                end
                if (cm_ix == 3'd7) begin
                    prev_above3 <= above[4];
                    state <= S_PASS;
                end
            end

            default: state <= S_PASS;
            endcase
        end
    end
endmodule
