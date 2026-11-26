// aac_parse -- the AAC-LC bitstream syntax, ADTS or raw, one bit a cycle.
//
// Reads ISO/IEC 14496-3 subpart 1's adts_frame() (or, with cfg_raw, one
// raw_data_block() per access unit, the end of a unit marked by in_last on
// its last byte) and subpart 4's raw_data_block() with the elements an
// AAC-LC mono or stereo stream carries: one SCE or one CPE, and any number
// of FIL, DSE and PCE, which are skipped. CCE and LFE are refused.
//
// What it produces, for the rest of the decoder:
//
//   * every spectral coefficient of every channel, exactly 1024 a channel,
//     de-interleaved (c_addr is window * 128 + bin, or bin for a long
//     window), with its band's scalefactor, after the pulses are added;
//     bands with no spectral data (zero, noise, intensity, and everything
//     from max_sfb up) come out as zeros with c_zero set;
//   * each band's codebook and value (scalefactor, intensity position or
//     noise energy) into the band RAM of its channel, and the ms_used bits;
//   * the TNS filters, each reduced to a window, a band range, a direction
//     and an order, and their coefficients as indexes into a sine table;
//   * the ics_info of each channel and the CPE's common fields.
//
// It waits for `go` before each raw_data_block, since what it writes is
// what the previous block's synthesis is still reading, and reports the
// end of the block with `blk_done` or a refusal with `blk_err`.
//
// Every field is read by S_GB: gb_n bits into gb_acc, then on to gb_ret.

module aac_parse (
    input  wire        clk,
    input  wire        rst_n,

    input  wire        cfg_raw,
    input  wire [3:0]  cfg_sfi,
    input  wire [1:0]  cfg_chan,

    input  wire        bit_avail,
    input  wire        bit_val,
    input  wire        bit_eob,
    input  wire        bit_last,
    output wire        bit_take,

    input  wire        go,
    output reg         blk_done,
    output reg         blk_err,
    output reg  [3:0]  err_code,
    output reg         idle,
    output wire        rdb_wait,

    output reg  [3:0]  sfi,
    output reg         cpe,
    output reg         common,
    output reg  [1:0]  ms_mask,
    output reg  [1:0]  ws0,
    output reg  [1:0]  ws1,
    output reg         shape0,
    output reg         shape1,
    output reg  [5:0]  msfb0,
    output reg  [5:0]  msfb1,
    output reg  [6:0]  grp0,
    output reg  [6:0]  grp1,
    output reg  [3:0]  ntns0,
    output reg  [3:0]  ntns1,

    output reg         band_we,
    output reg         band_ch,
    output reg  [6:0]  band_addr,
    output reg  [3:0]  band_cb,
    output reg  [11:0] band_val,
    output reg         ms_we,
    output reg  [6:0]  ms_addr,
    output reg         ms_bit,

    output reg         td_we,
    output reg         td_ch,
    output reg  [2:0]  td_idx,
    output reg  [19:0] td_data,
    output reg         tc_we,
    output reg         tc_ch,
    output reg  [6:0]  tc_addr,
    output reg  [4:0]  tc_data,

    output reg         c_valid,
    output reg         c_ch,
    output reg  [9:0]  c_addr,
    output reg  [14:0] c_q,
    output reg  [7:0]  c_sf,
    output reg         c_zero
);
    // ------------------------------------------------------------------
    // States
    // ------------------------------------------------------------------
    localparam [6:0]
        S_IDLE    = 7'd0,   S_GB      = 7'd1,   S_SKIP    = 7'd2,
        S_SYNC    = 7'd3,   S_SYNC1   = 7'd4,   S_SYNC2   = 7'd5,
        S_HDR1    = 7'd6,   S_HDR2    = 7'd7,   S_HDR3    = 7'd8,
        S_HDR4    = 7'd9,   S_RDB     = 7'd10,  S_ID      = 7'd11,
        S_ID2     = 7'd12,  S_SCE     = 7'd13,  S_CPE     = 7'd14,
        S_FIL     = 7'd15,  S_FIL2    = 7'd16,  S_DSE     = 7'd17,
        S_DSE2    = 7'd18,  S_DSE3    = 7'd19,  S_PCE     = 7'd20,
        S_PCE2    = 7'd21,  S_PCE3    = 7'd22,  S_PCE4    = 7'd23,
        S_PCE5    = 7'd24,  S_PCE6    = 7'd25,  S_PCE7    = 7'd26,
        S_ALIGN   = 7'd27,  S_END     = 7'd28,  S_END2    = 7'd29,
        S_TAILSK  = 7'd30,  S_ERR     = 7'd31,  S_ERR2    = 7'd32,
        S_ICS     = 7'd33,  S_ICS2    = 7'd34,  S_INFO    = 7'd35,
        S_INFO2   = 7'd36,  S_INFO3   = 7'd37,  S_MS      = 7'd38,
        S_MSB     = 7'd39,  S_MSB2    = 7'd40,  S_SEC     = 7'd41,
        S_SEC2    = 7'd42,  S_SEC3    = 7'd43,  S_SEC4    = 7'd44,
        S_SF      = 7'd45,  S_SF2     = 7'd46,  S_SF3     = 7'd47,
        S_SF4     = 7'd48,  S_PUL     = 7'd49,  S_PUL2    = 7'd50,
        S_PUL3    = 7'd51,  S_PUL4    = 7'd52,  S_TNS     = 7'd53,
        S_TNSW    = 7'd54,  S_TNSN    = 7'd55,  S_TNSR    = 7'd56,
        S_TNSF    = 7'd57,  S_TNSF2   = 7'd58,  S_TNSF3   = 7'd59,
        S_TNSC    = 7'd60,  S_TNSC2   = 7'd61,  S_TNSD    = 7'd62,
        S_GAIN    = 7'd63,  S_GAIN2   = 7'd64,  S_SP      = 7'd65,
        S_SPB     = 7'd66,  S_SPB2    = 7'd67,  S_SPW     = 7'd68,
        S_SPZ     = 7'd69,  S_SPC     = 7'd70,  S_SPCW    = 7'd71,
        S_SPS     = 7'd72,  S_SPE     = 7'd73,  S_SPEP    = 7'd74,
        S_SPEW    = 7'd75,  S_SPO     = 7'd76,  S_SPT     = 7'd77,
        S_SPT2    = 7'd78,  S_ICSEND  = 7'd79,  S_SF5     = 7'd80,
        S_TNSF4   = 7'd81,  S_CPE2    = 7'd82,  S_SEC5    = 7'd83,
        S_PULW    = 7'd84,  S_SPTW    = 7'd85;

    reg [6:0] state;
    // Ready for a raw_data_block and waiting for `go`: the sequencer takes
    // the spectral buffers' ownership away from synthesis on go && rdb_wait.
    assign rdb_wait = state == S_RDB;

    // ------------------------------------------------------------------
    // The field reader
    // ------------------------------------------------------------------
    reg [4:0]  gb_n;
    reg [15:0] gb_acc;
    reg [6:0]  gb_ret;
    reg [15:0] skip_n;     // bits left to skip in S_SKIP
    reg [6:0]  skip_ret;

    // Where the data ends: the ADTS frame's length, or the access unit's
    // last byte.
    reg [15:0] bitcnt;     // bits taken since the ADTS syncword
    reg [12:0] flen;
    reg        hdr_ok;     // flen is known
    reg        au_end;     // the byte with in_last has been taken whole
    reg        in_rdb;     // reading inside a raw_data_block
    wire [15:0] fbits = {flen, 3'b000};
    wire       at_end = cfg_raw ? au_end : (hdr_ok && bitcnt == fbits);
    wire       can_take = bit_avail && !(in_rdb && at_end);

    // Huffman decoder
    reg        h_start;
    reg [3:0]  h_book;
    wire       h_take;
    wire       h_hit;
    wire [9:0] h_pay;
    aac_huff u_huff (
        .clk(clk), .rst_n(rst_n), .start(h_start), .abort(state == S_ERR),
        .book(h_book),
        .bit_avail(can_take), .bit_val(bit_val), .bit_take(h_take),
        .hit(h_hit), .payload(h_pay)
    );
    reg h_busy;

    reg  p_take;   // the parser's own take, decided below
    assign bit_take = p_take || h_take;
    wire taken = bit_take && bit_avail;

    // ------------------------------------------------------------------
    // Frame and element state
    // ------------------------------------------------------------------
    reg        lost;        // searching for a syncword after losing one
    reg        prot_absent;
    reg [1:0]  nrdb;
    reg [1:0]  rdb_i;
    reg [1:0]  chan;
    reg        seen_audio;
    reg        ch;          // channel being parsed
    reg [7:0]  cnt8;        // general counters
    reg [8:0]  cnt9;
    reg [3:0]  pce_nf, pce_ns, pce_nb, pce_nc;
    reg [1:0]  pce_nl;
    reg [2:0]  pce_na;
    // The element lists of a PCE: 5 bits a front, side, back or coupling
    // element, 4 an LFE or data element (Table 4.2).
    wire [7:0]  pce_n5 = {4'd0, pce_nf} + {4'd0, pce_ns} + {4'd0, pce_nb} + {4'd0, pce_nc};
    wire [7:0]  pce_n4 = {6'd0, pce_nl} + {5'd0, pce_na};
    wire [15:0] pce_bits = {6'd0, pce_n5, 2'd0} + {8'd0, pce_n5} + {6'd0, pce_n4, 2'd0};

    // The ics_info of the channel being parsed.
    reg [1:0]  ws;
    reg        shp;
    reg [5:0]  msfb;
    reg [6:0]  grp;
    wire       short_w = ws == 2'd2;
    wire [3:0] nwin = short_w ? 4'd8 : 4'd1;

    // Groups: g, its first window and its length.
    reg [2:0]  g;
    reg [3:0]  w0;
    reg [3:0]  glen;
    function [3:0] glen_f;
        input [6:0] gp;
        input [3:0] start;
        input       sw;
        reg   [3:0] n;
        reg         run;
        integer     w;
        begin
            n = 4'd1;
            run = 1'b1;
            if (sw) begin
                for (w = 1; w < 8; w = w + 1) begin
                    if (w > start) begin
                        if (run && gp[7 - w])
                            n = n + 4'd1;
                        else
                            run = 1'b0;
                    end
                end
            end
            glen_f = n;
        end
    endfunction

    // Band tables: the offsets of band_a and the next, a cycle after
    // band_a is set.
    reg  [5:0]  band_a;
    wire [10:0] off_a, off_b;
    wire [5:0]  num_swb, tns_max;
    aac_swb u_swb (.clk(clk), .sfi(sfi), .short_win(short_w),
                   .band_a(band_a), .band_b(band_a + 6'd1),
                   .off_a(off_a), .off_b(off_b),
                   .num_swb(num_swb), .tns_max(tns_max));

    // The parser's own copy of each band's codebook and scalefactor.
    reg  [11:0] lb_mem [0:127];
    reg         lb_we;
    reg  [6:0]  lb_waddr;
    reg  [11:0] lb_wdata;
    reg  [6:0]  lb_raddr;
    reg  [11:0] lb_q;
    always @(posedge clk) begin
        if (lb_we)
            lb_mem[lb_waddr] <= lb_wdata;
        lb_q <= lb_mem[lb_raddr];
    end
    wire [6:0] baddr = short_w ? {g, band_a[3:0]} : {1'b0, band_a};

    // Sections
    reg  [3:0]  sec_cb;
    reg  [5:0]  sec_k;      // first band of the section
    reg  [6:0]  sec_len;    // grows by escapes, checked against max_sfb

    // Scalefactors
    reg  [7:0]  gain;
    reg  [8:0]  sf_acc;     // 0..255, checked
    reg  [11:0] is_acc;
    reg  [11:0] nrg_acc;
    reg         noise_pcm;
    reg  [3:0]  cur_cb;
    reg  [7:0]  cur_sf;
    // The next scalefactor, as ten bits two's complement: -60..315.
    wire [9:0]  sf_next = {2'd0, sf_acc[7:0]} + {3'd0, h_pay[6:0]} - 10'd60;

    // Pulses
    reg         pul_on;
    reg  [1:0]  pul_num;
    reg  [1:0]  pul_i;
    reg  [10:0] pul_k;
    reg  [10:0] pul_pos0, pul_pos1, pul_pos2, pul_pos3;
    reg  [3:0]  pul_amp0, pul_amp1, pul_amp2, pul_amp3;
    reg  [3:0]  pul_cnt;    // how many are set

    // TNS
    reg  [2:0]  tw;         // window
    reg  [1:0]  tnf;        // filters in this window
    reg  [1:0]  tf;
    reg         tres;
    reg  [5:0]  tlen;
    reg  [4:0]  tord;
    reg         tdir;
    reg         tcomp;
    reg  [4:0]  ti;
    reg  [5:0]  tbot;       // bottom of the previous filter (band)
    reg  [3:0]  tcount;     // descriptors written this channel

    // Spectral data
    reg  [10:0] base;       // coefficient address of the band in the window
    reg  [6:0]  width;
    reg  [6:0]  bin;
    reg  [3:0]  win;
    reg  [1:0]  d;
    reg  [2:0]  dim;
    reg  [14:0] v0, v1, v2, v3;
    reg  [3:0]  esc_n;
    reg  [10:0] tail_k;

    wire cb_unsigned = (cur_cb == 4'd3) || (cur_cb == 4'd4) || (cur_cb >= 4'd7);
    wire cb_quad = cur_cb <= 4'd4;
    wire cb_none = (cur_cb == 4'd0) || (cur_cb >= 4'd13);

    reg [14:0] vd;
    always @(*) begin
        case (d)
            2'd0: vd = v0;
            2'd1: vd = v1;
            2'd2: vd = v2;
            default: vd = v3;
        endcase
    end

    // A coefficient with its pulses (4.6.3.3): each pulse at this address
    // in order, adding to a positive value and subtracting otherwise.
    function [14:0] pulse_add;
        input [14:0] q;
        input        hit;
        input [3:0]  amp;
        begin
            if (!hit)
                pulse_add = q;
            else if (!q[14] && q != 15'd0)
                pulse_add = q + {11'd0, amp};
            else
                pulse_add = q - {11'd0, amp};
        end
    endfunction
    wire [10:0] emit_addr = base + {4'd0, bin} + {9'd0, d};
    wire [14:0] pq0 = pulse_add(vd,  pul_on && pul_cnt > 4'd0 && pul_pos0 == emit_addr, pul_amp0);
    wire [14:0] pq1 = pulse_add(pq0, pul_on && pul_cnt > 4'd1 && pul_pos1 == emit_addr, pul_amp1);
    wire [14:0] pq2 = pulse_add(pq1, pul_on && pul_cnt > 4'd2 && pul_pos2 == emit_addr, pul_amp2);
    wire [14:0] pq3 = pulse_add(pq2, pul_on && pul_cnt > 4'd3 && pul_pos3 == emit_addr, pul_amp3);

    // Sign extension of a TNS coefficient of res2 bits to four.
    function [3:0] tns_sext;
        input [3:0] c;
        input [2:0] bits;
        begin
            case (bits)
                3'd2: tns_sext = {{2{c[1]}}, c[1:0]};
                3'd3: tns_sext = {c[2], c[2:0]};
                default: tns_sext = c;
            endcase
        end
    endfunction

    // A section length escape and field width (4.4.2.7, section_data).
    wire [5:0] sec_esc  = short_w ? 6'd7 : 6'd31;
    wire [4:0] sec_bits = short_w ? 5'd3 : 5'd5;

    // ------------------------------------------------------------------
    // Helpers for the state machine
    // ------------------------------------------------------------------
    task getbits;
        input [4:0] n;
        input [6:0] ret;
        begin
            gb_n   <= n;
            gb_acc <= 16'd0;
            gb_ret <= ret;
            state  <= S_GB;
        end
    endtask

    task skipbits;
        input [15:0] n;
        input [6:0]  ret;
        begin
            skip_n   <= n;
            skip_ret <= ret;
            state    <= S_SKIP;
        end
    endtask

    task fail;
        input [3:0] code;
        begin
            err_code <= code;
            state    <= S_ERR;
        end
    endtask

    task huff;
        input [3:0] book;
        begin
            h_start <= 1'b1;
            h_book  <= book;
            h_busy  <= 1'b1;
        end
    endtask

    // The parser takes a bit in S_GB, S_SKIP, the alignment and the
    // one-bit loops; everything else reads through S_GB.
    always @(*) begin
        p_take = 1'b0;
        case (state)
            S_GB:    p_take = can_take;
            S_SKIP:  p_take = can_take && skip_n != 16'd0;
            S_ALIGN: p_take = can_take && bitcnt[2:0] != 3'd0;
            S_TAILSK: p_take = bit_avail && (cfg_raw ? !au_end : bitcnt != fbits);
            S_ERR2:  p_take = bit_avail && (cfg_raw ? !au_end
                                                   : (hdr_ok && bitcnt < fbits) || bitcnt[2:0] != 3'd0);
            S_SPS:   p_take = can_take && vd != 15'd0 && d < dim;
            S_SPEP:  p_take = can_take;
            default: p_take = 1'b0;
        endcase
    end

    // ------------------------------------------------------------------
    // The state machine
    // ------------------------------------------------------------------
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state <= S_IDLE;
            gb_n <= 5'd0; gb_acc <= 16'd0; gb_ret <= S_IDLE;
            skip_n <= 16'd0; skip_ret <= S_IDLE;
            bitcnt <= 16'd0; flen <= 13'd0; hdr_ok <= 1'b0; au_end <= 1'b0;
            in_rdb <= 1'b0;
            h_start <= 1'b0; h_book <= 4'd0; h_busy <= 1'b0;
            blk_done <= 1'b0; blk_err <= 1'b0; err_code <= 4'd0; idle <= 1'b1;
            sfi <= 4'd4; cpe <= 1'b0; common <= 1'b0; ms_mask <= 2'd0;
            ws0 <= 2'd0; ws1 <= 2'd0; shape0 <= 1'b0; shape1 <= 1'b0;
            msfb0 <= 6'd0; msfb1 <= 6'd0; grp0 <= 7'd0; grp1 <= 7'd0;
            ntns0 <= 4'd0; ntns1 <= 4'd0;
            band_we <= 1'b0; band_ch <= 1'b0; band_addr <= 7'd0;
            band_cb <= 4'd0; band_val <= 12'd0;
            ms_we <= 1'b0; ms_addr <= 7'd0; ms_bit <= 1'b0;
            td_we <= 1'b0; td_ch <= 1'b0; td_idx <= 3'd0; td_data <= 20'd0;
            tc_we <= 1'b0; tc_ch <= 1'b0; tc_addr <= 7'd0; tc_data <= 5'd0;
            c_valid <= 1'b0; c_ch <= 1'b0; c_addr <= 10'd0; c_q <= 15'd0;
            c_sf <= 8'd0; c_zero <= 1'b0;
            lost <= 1'b0; prot_absent <= 1'b1; nrdb <= 2'd0; rdb_i <= 2'd0; chan <= 2'd1;
            seen_audio <= 1'b0; ch <= 1'b0; cnt8 <= 8'd0; cnt9 <= 9'd0;
            pce_nf <= 4'd0; pce_ns <= 4'd0; pce_nb <= 4'd0; pce_nc <= 4'd0;
            pce_nl <= 2'd0; pce_na <= 3'd0;
            ws <= 2'd0; shp <= 1'b0; msfb <= 6'd0; grp <= 7'd0;
            g <= 3'd0; w0 <= 4'd0; glen <= 4'd1;
            band_a <= 6'd0;
            lb_we <= 1'b0; lb_waddr <= 7'd0; lb_wdata <= 12'd0; lb_raddr <= 7'd0;
            sec_cb <= 4'd0; sec_k <= 6'd0; sec_len <= 7'd0;
            gain <= 8'd0; sf_acc <= 9'd0; is_acc <= 12'd0; nrg_acc <= 12'd0;
            noise_pcm <= 1'b1; cur_cb <= 4'd0; cur_sf <= 8'd0;
            pul_on <= 1'b0; pul_num <= 2'd0; pul_i <= 2'd0; pul_k <= 11'd0;
            pul_pos0 <= 11'd0; pul_pos1 <= 11'd0; pul_pos2 <= 11'd0; pul_pos3 <= 11'd0;
            pul_amp0 <= 4'd0; pul_amp1 <= 4'd0; pul_amp2 <= 4'd0; pul_amp3 <= 4'd0;
            pul_cnt <= 4'd0;
            tw <= 3'd0; tnf <= 2'd0; tf <= 2'd0; tres <= 1'b0; tlen <= 6'd0;
            tord <= 5'd0; tdir <= 1'b0; tcomp <= 1'b0; ti <= 5'd0; tbot <= 6'd0;
            tcount <= 4'd0;
            base <= 11'd0; width <= 7'd0; bin <= 7'd0; win <= 4'd0; d <= 2'd0;
            dim <= 3'd0; v0 <= 15'd0; v1 <= 15'd0; v2 <= 15'd0; v3 <= 15'd0;
            esc_n <= 4'd0; tail_k <= 11'd0;
        end else begin
            // One-cycle strobes.
            blk_done <= 1'b0;
            blk_err  <= 1'b0;
            h_start  <= 1'b0;
            band_we  <= 1'b0;
            ms_we    <= 1'b0;
            td_we    <= 1'b0;
            tc_we    <= 1'b0;
            c_valid  <= 1'b0;
            lb_we    <= 1'b0;

            // Bit accounting, whoever took the bit.
            if (taken) begin
                bitcnt <= bitcnt + 16'd1;
                if (bit_eob && bit_last)
                    au_end <= 1'b1;
            end

            case (state)
            // ----------------------------------------------------------
            S_GB: begin
                if (bit_avail && in_rdb && at_end) begin
                    fail(cfg_raw ? 4'd10 : 4'd9);
                end else if (can_take) begin
                    gb_acc <= {gb_acc[14:0], bit_val};
                    gb_n   <= gb_n - 5'd1;
                    if (gb_n == 5'd1)
                        state <= gb_ret;
                end
            end
            S_SKIP: begin
                if (skip_n == 16'd0)
                    state <= skip_ret;
                else if (bit_avail && in_rdb && at_end)
                    fail(cfg_raw ? 4'd10 : 4'd9);
                else if (can_take)
                    skip_n <= skip_n - 16'd1;
            end
            S_ALIGN: begin
                if (bitcnt[2:0] == 3'd0)
                    state <= gb_ret;
                else if (bit_avail && in_rdb && at_end)
                    fail(cfg_raw ? 4'd10 : 4'd9);
            end

            // ----------------------------------------------------------
            // Frame start
            S_IDLE: begin
                idle <= 1'b1;
                bitcnt <= 16'd0;
                hdr_ok <= 1'b0;
                au_end <= 1'b0;
                in_rdb <= 1'b0;
                if (cfg_raw) begin
                    if (bit_avail) begin
                        sfi    <= cfg_sfi;
                        chan   <= cfg_chan;
                        nrdb   <= 2'd0;
                        rdb_i  <= 2'd0;
                        prot_absent <= 1'b1;
                        if (cfg_sfi < 4'd3 || cfg_sfi > 4'd12 ||
                            cfg_chan == 2'd0 || cfg_chan == 2'd3)
                            fail(4'd2);
                        else
                            state <= S_RDB;
                    end
                end else if (bit_avail) begin
                    state <= S_SYNC;
                end
            end
            S_SYNC: begin
                // Byte-aligned search for 0xFFF.
                bitcnt <= 16'd0;
                getbits(5'd8, S_SYNC1);
            end
            S_SYNC1: begin
                if (gb_acc[7:0] == 8'hFF)
                    getbits(5'd4, S_SYNC2);
                else begin
                    // Reported once per loss, then searched in silence.
                    if (!lost) begin
                        lost <= 1'b1;
                        blk_err <= 1'b1;
                        err_code <= 4'd1;
                    end
                    state <= S_SYNC;
                end
            end
            S_SYNC2: begin
                if (gb_acc[3:0] == 4'hF) begin
                    getbits(5'd16, S_HDR1);
                end else begin
                    if (!lost) begin
                        lost <= 1'b1;
                        blk_err <= 1'b1;
                        err_code <= 4'd1;
                    end
                    gb_ret <= S_SYNC;
                    state  <= S_ALIGN;
                end
            end
            S_HDR1: begin
                lost <= 1'b0;
                // ID layer protection_absent profile sfi private channel
                // original home: 1 2 1 2 4 1 3 1 1.
                prot_absent <= gb_acc[12];
                sfi  <= gb_acc[9:6];
                chan <= gb_acc[3:2];
                if (gb_acc[11:10] != 2'd1 || gb_acc[9:6] < 4'd3 ||
                    gb_acc[9:6] > 4'd12 || gb_acc[4] ||
                    gb_acc[3:2] == 2'd0 || gb_acc[3:2] == 2'd3)
                    fail(4'd2);
                else
                    getbits(5'd15, S_HDR2);
            end
            S_HDR2: begin
                // copyright bits, aac_frame_length
                flen   <= gb_acc[12:0];
                hdr_ok <= 1'b1;
                getbits(5'd13, S_HDR3);
            end
            S_HDR3: begin
                // adts_buffer_fullness, number_of_raw_data_blocks_in_frame
                nrdb  <= gb_acc[1:0];
                rdb_i <= 2'd0;
                if (!prot_absent)
                    skipbits({gb_acc[1:0], 4'd0} + 16'd16, S_RDB);
                else
                    state <= S_RDB;
            end

            // ----------------------------------------------------------
            // raw_data_block
            S_RDB: begin
                idle <= 1'b0;
                if (go) begin
                    in_rdb <= 1'b1;
                    seen_audio <= 1'b0;
                    ntns0 <= 4'd0;
                    ntns1 <= 4'd0;
                    state <= S_ID;
                end
            end
            S_ID: getbits(5'd3, S_ID2);
            S_ID2: begin
                case (gb_acc[2:0])
                    3'd0: if (seen_audio || chan != 2'd1) fail(4'd3);
                          else getbits(5'd4, S_SCE);
                    3'd1: if (seen_audio || chan != 2'd2) fail(4'd3);
                          else getbits(5'd5, S_CPE);
                    3'd4: getbits(5'd13, S_DSE);
                    3'd5: getbits(5'd10, S_PCE);
                    3'd6: getbits(5'd4, S_FIL);
                    3'd7: begin gb_ret <= S_END; state <= S_ALIGN; end
                    default: fail(4'd3);
                endcase
            end
            S_SCE: begin
                seen_audio <= 1'b1;
                cpe    <= 1'b0;
                common <= 1'b0;
                ms_mask <= 2'd0;
                ch     <= 1'b0;
                getbits(5'd8, S_ICS);
            end
            S_CPE: begin
                seen_audio <= 1'b1;
                cpe    <= 1'b1;
                common <= gb_acc[0];
                ms_mask <= 2'd0;
                ch     <= 1'b0;
                if (gb_acc[0]) begin
                    getbits(5'd4, S_INFO);
                end else
                    getbits(5'd8, S_ICS);
            end
            S_CPE2: begin
                // After the common ics_info: ms_mask_present.
                getbits(5'd2, S_MS);
            end
            S_MS: begin
                ms_mask <= gb_acc[1:0];
                if (gb_acc[1:0] == 2'd3)
                    fail(4'd4);
                else if (gb_acc[1:0] == 2'd1) begin
                    g <= 3'd0; w0 <= 4'd0; glen <= glen_f(grp, 4'd0, short_w);
                    band_a <= 6'd0;
                    state <= S_MSB;
                end else
                    getbits(5'd8, S_ICS);
            end
            S_MSB: begin
                if (w0 == nwin)
                    getbits(5'd8, S_ICS);
                else if (band_a == msfb) begin
                    g <= g + 3'd1;
                    w0 <= w0 + glen;
                    glen <= glen_f(grp, w0 + glen, short_w);
                    band_a <= 6'd0;
                end else
                    getbits(5'd1, S_MSB2);
            end
            S_MSB2: begin
                ms_we   <= 1'b1;
                ms_addr <= baddr;
                ms_bit  <= gb_acc[0];
                band_a  <= band_a + 6'd1;
                state   <= S_MSB;
            end

            // ----------------------------------------------------------
            // fill_element, data_stream_element, program_config_element
            S_FIL: begin
                if (gb_acc[3:0] == 4'd15)
                    getbits(5'd8, S_FIL2);
                else
                    skipbits({9'd0, gb_acc[3:0], 3'd0}, S_ID);
            end
            // cnt = 15 + esc_count - 1
            S_FIL2: skipbits({4'd0, {1'b0, gb_acc[7:0]} + 9'd14, 3'd0}, S_ID);
            S_DSE: begin
                // tag(4) align(1) count(8)
                cnt9 <= {1'b0, gb_acc[7:0]};
                cnt8 <= {7'd0, gb_acc[8]};
                if (gb_acc[7:0] == 8'd255)
                    getbits(5'd8, S_DSE2);
                else
                    state <= S_DSE3;
            end
            S_DSE2: begin
                cnt9 <= 9'd255 + {1'b0, gb_acc[7:0]};
                state <= S_DSE3;
            end
            S_DSE3: begin
                if (cnt8[0]) begin
                    cnt8[0] <= 1'b0;
                    gb_ret <= S_DSE3;
                    state <= S_ALIGN;
                end else
                    skipbits({4'd0, cnt9, 3'd0}, S_ID);
            end
            S_PCE: getbits(5'd12, S_PCE2);   // nf ns nb
            S_PCE2: begin
                pce_nf <= gb_acc[11:8];
                pce_ns <= gb_acc[7:4];
                pce_nb <= gb_acc[3:0];
                getbits(5'd9, S_PCE3);        // nl na nc
            end
            S_PCE3: begin
                pce_nl <= gb_acc[8:7];
                pce_na <= gb_acc[6:4];
                pce_nc <= gb_acc[3:0];
                getbits(5'd1, S_PCE4);
            end
            S_PCE4: begin                     // mono_mixdown_present
                cnt8 <= 8'd0;
                if (gb_acc[0]) getbits(5'd5, S_PCE5);   // + stereo present
                else getbits(5'd1, S_PCE5);
            end
            S_PCE5: begin                     // stereo_mixdown_present
                if (gb_acc[0]) getbits(5'd5, S_PCE6);   // + matrix present
                else getbits(5'd1, S_PCE6);
            end
            S_PCE6: begin                     // matrix_mixdown_idx_present
                if (gb_acc[0]) skipbits(16'd3, S_PCE7);
                else state <= S_PCE7;
            end
            S_PCE7: begin
                if (cnt8 == 8'd0) begin
                    // the element lists, then byte_alignment
                    cnt8 <= 8'd1;
                    skipbits(pce_bits, S_PCE7);
                end else if (cnt8 == 8'd1) begin
                    cnt8 <= 8'd2;
                    gb_ret <= S_PCE7;
                    state <= S_ALIGN;
                end else if (cnt8 == 8'd2) begin
                    cnt8 <= 8'd3;
                    getbits(5'd8, S_PCE7);
                end else
                    skipbits({5'd0, gb_acc[7:0], 3'd0}, S_ID);
            end

            // ----------------------------------------------------------
            // End of the raw_data_block
            S_END: begin
                if (!seen_audio)
                    fail(4'd3);
                else begin
                    in_rdb <= 1'b0;
                    blk_done <= 1'b1;
                    state <= S_END2;
                end
            end
            S_END2: begin
                if (!cfg_raw && rdb_i != nrdb) begin
                    rdb_i <= rdb_i + 2'd1;
                    if (!prot_absent)
                        skipbits(16'd16, S_RDB);
                    else
                        state <= S_RDB;
                end else if (!cfg_raw && !prot_absent && nrdb != 2'd0) begin
                    skipbits(16'd16, S_TAILSK);
                end else
                    state <= S_TAILSK;
            end
            S_TAILSK: begin
                // To the end of the ADTS frame or of the access unit.
                if (cfg_raw ? au_end : (bitcnt >= fbits))
                    state <= S_IDLE;
            end
            S_ERR: begin
                blk_err <= 1'b1;
                in_rdb <= 1'b0;
                h_busy <= 1'b0;
                state <= S_ERR2;
            end
            S_ERR2: begin
                // Skip what is left of the frame or unit, then look again.
                if (cfg_raw) begin
                    if (au_end) state <= S_IDLE;
                end else if (!((hdr_ok && bitcnt < fbits) || bitcnt[2:0] != 3'd0))
                    state <= S_IDLE;
            end

            // ----------------------------------------------------------
            // individual_channel_stream
            S_ICS: begin
                gain <= gb_acc[7:0];
                if (!common)
                    getbits(5'd4, S_INFO);
                else
                    state <= S_ICS2;
            end
            S_INFO: begin
                // ics_reserved_bit window_sequence window_shape
                ws  <= gb_acc[2:1];
                shp <= gb_acc[0];
                if (gb_acc[3])
                    fail(4'd4);
                else if (gb_acc[2:1] == 2'd2)
                    getbits(5'd11, S_INFO2);
                else
                    getbits(5'd7, S_INFO2);
            end
            S_INFO2: begin
                if (short_w) begin
                    msfb <= {2'd0, gb_acc[10:7]};
                    grp  <= gb_acc[6:0];
                end else begin
                    msfb <= gb_acc[6:1];
                    grp  <= 7'd0;
                end
                band_a <= 6'd0;
                if (!short_w && gb_acc[0])
                    fail(4'd4);            // predictor_data_present
                else
                    state <= S_INFO3;
            end
            S_INFO3: begin
                if (msfb > num_swb)
                    fail(4'd5);
                else begin
                    // Both channels of a common window take this ics_info.
                    if (!ch || common) begin
                        ws0 <= ws; shape0 <= shp; msfb0 <= msfb; grp0 <= grp;
                    end
                    if (ch || common) begin
                        ws1 <= ws; shape1 <= shp; msfb1 <= msfb; grp1 <= grp;
                    end
                    // A common ics_info is the CPE's, before its
                    // ms_mask; otherwise it is the ICS's own.
                    if (common)
                        state <= S_CPE2;
                    else
                        state <= S_ICS2;
                end
            end
            S_ICS2: begin
                // section_data
                g <= 3'd0; w0 <= 4'd0; glen <= glen_f(grp, 4'd0, short_w);
                sec_k <= 6'd0;
                state <= S_SEC;
            end
            S_SEC: begin
                if (w0 == nwin) begin
                    // scale_factor_data
                    g <= 3'd0; w0 <= 4'd0; glen <= glen_f(grp, 4'd0, short_w);
                    band_a <= 6'd0;
                    sf_acc <= {1'b0, gain};
                    is_acc <= 12'd0;
                    nrg_acc <= {4'd0, gain} - 12'd346;
                    noise_pcm <= 1'b1;
                    state <= S_SF;
                end else if (sec_k == msfb) begin
                    g <= g + 3'd1;
                    w0 <= w0 + glen;
                    glen <= glen_f(grp, w0 + glen, short_w);
                    sec_k <= 6'd0;
                end else
                    getbits(5'd4, S_SEC2);
            end
            S_SEC2: begin
                sec_cb <= gb_acc[3:0];
                sec_len <= 7'd0;
                if (gb_acc[3:0] == 4'd12)
                    fail(4'd5);
                else
                    getbits(sec_bits, S_SEC3);
            end
            S_SEC3: begin
                if (gb_acc[5:0] == sec_esc) begin
                    sec_len <= sec_len + {1'b0, sec_esc};
                    if ({1'b0, sec_k} + sec_len + {1'b0, sec_esc} > {1'b0, msfb})
                        fail(4'd5);
                    else
                        getbits(sec_bits, S_SEC3);
                end else begin
                    sec_len <= sec_len + {1'b0, gb_acc[5:0]};
                    state <= S_SEC4;
                end
            end
            S_SEC4: begin
                if (sec_len == 7'd0 || {1'b0, sec_k} + sec_len > {1'b0, msfb})
                    fail(4'd5);
                else begin
                    band_a <= sec_k;
                    state <= S_SEC5;
                end
            end
            S_SEC5: begin
                // Write the section's codebook into every band of it.
                lb_we    <= 1'b1;
                lb_waddr <= baddr;
                lb_wdata <= {sec_cb, 8'd0};
                band_a   <= band_a + 6'd1;
                if ({1'b0, band_a} + 7'd1 == {1'b0, sec_k} + sec_len) begin
                    sec_k <= sec_k + sec_len[5:0];
                    state <= S_SEC;
                end
            end

            // scale_factor_data
            S_SF: begin
                if (w0 == nwin) begin
                    getbits(5'd1, S_PUL);
                end else if (band_a == msfb) begin
                    g <= g + 3'd1;
                    w0 <= w0 + glen;
                    glen <= glen_f(grp, w0 + glen, short_w);
                    band_a <= 6'd0;
                end else begin
                    lb_raddr <= baddr;
                    state <= S_SF2;
                end
            end
            S_SF2: state <= S_SF3;          // the read
            S_SF3: begin
                cur_cb <= lb_q[11:8];
                if (lb_q[11:8] == 4'd0) begin
                    band_we <= 1'b1; band_ch <= ch; band_addr <= baddr;
                    band_cb <= 4'd0; band_val <= 12'd0;
                    band_a <= band_a + 6'd1;
                    state <= S_SF;
                end else if (lb_q[11:8] == 4'd13 && noise_pcm) begin
                    noise_pcm <= 1'b0;
                    getbits(5'd9, S_SF5);
                end else begin
                    huff(4'd0);
                    state <= S_SF4;
                end
            end
            S_SF4: begin
                if (h_busy && !h_start && h_hit) begin
                    h_busy <= 1'b0;
                    band_we <= 1'b1; band_ch <= ch; band_addr <= baddr;
                    band_cb <= cur_cb;
                    band_a <= band_a + 6'd1;
                    state <= S_SF;
                    if (cur_cb == 4'd14 || cur_cb == 4'd15) begin
                        is_acc <= is_acc + {5'd0, h_pay[6:0]} - 12'd60;
                        band_val <= is_acc + {5'd0, h_pay[6:0]} - 12'd60;
                    end else if (cur_cb == 4'd13) begin
                        nrg_acc <= nrg_acc + {5'd0, h_pay[6:0]} - 12'd60;
                        band_val <= nrg_acc + {5'd0, h_pay[6:0]} - 12'd60;
                    end else begin
                        sf_acc <= {1'b0, sf_next[7:0]};
                        band_val <= {4'd0, sf_next[7:0]};
                        lb_we <= 1'b1;
                        lb_waddr <= baddr;
                        lb_wdata <= {cur_cb, sf_next[7:0]};
                        // 0..255 (4.6.2.3.2)
                        if (sf_next[9] || sf_next[8])
                            fail(4'd6);
                    end
                end else if (in_rdb && at_end && bit_avail)
                    fail(cfg_raw ? 4'd10 : 4'd9);
            end
            S_SF5: begin
                band_we <= 1'b1; band_ch <= ch; band_addr <= baddr;
                band_cb <= 4'd13;
                nrg_acc <= nrg_acc + {3'd0, gb_acc[8:0]};
                band_val <= nrg_acc + {3'd0, gb_acc[8:0]};
                band_a <= band_a + 6'd1;
                state <= S_SF;
            end

            // pulse_data
            S_PUL: begin
                pul_on <= gb_acc[0];
                pul_cnt <= 4'd0;
                if (gb_acc[0]) begin
                    if (short_w)
                        fail(4'd7);
                    else
                        getbits(5'd8, S_PUL2);
                end else
                    getbits(5'd1, S_TNS);
            end
            S_PUL2: begin
                pul_num <= gb_acc[7:6];
                pul_i <= 2'd0;
                band_a <= gb_acc[5:0];
                if (gb_acc[5:0] > num_swb)
                    fail(4'd7);
                else
                    state <= S_PULW;
            end
            S_PULW: state <= S_PUL3;     // the band table's read
            S_PUL3: begin
                pul_k <= off_a;
                getbits(5'd9, S_PUL4);
            end
            S_PUL4: begin
                if (pul_k + {6'd0, gb_acc[8:4]} > 11'd1023)
                    fail(4'd7);
                else begin
                    pul_k <= pul_k + {6'd0, gb_acc[8:4]};
                    pul_cnt <= pul_cnt + 4'd1;
                    case (pul_i)
                        2'd0: begin pul_pos0 <= pul_k + {6'd0, gb_acc[8:4]}; pul_amp0 <= gb_acc[3:0]; end
                        2'd1: begin pul_pos1 <= pul_k + {6'd0, gb_acc[8:4]}; pul_amp1 <= gb_acc[3:0]; end
                        2'd2: begin pul_pos2 <= pul_k + {6'd0, gb_acc[8:4]}; pul_amp2 <= gb_acc[3:0]; end
                        default: begin pul_pos3 <= pul_k + {6'd0, gb_acc[8:4]}; pul_amp3 <= gb_acc[3:0]; end
                    endcase
                    if (pul_i == pul_num)
                        getbits(5'd1, S_TNS);
                    else begin
                        pul_i <= pul_i + 2'd1;
                        getbits(5'd9, S_PUL4);
                    end
                end
            end

            // tns_data
            S_TNS: begin
                tcount <= 4'd0;
                if (gb_acc[0]) begin
                    tw <= 3'd0;
                    state <= S_TNSW;
                end else
                    getbits(5'd1, S_GAIN);
            end
            S_TNSW: begin
                // n_filt[w]
                tbot <= num_swb;
                getbits(short_w ? 5'd1 : 5'd2, S_TNSN);
            end
            S_TNSN: begin
                tnf <= gb_acc[1:0];
                tf <= 2'd0;
                if (gb_acc[1:0] != 2'd0)
                    getbits(5'd1, S_TNSR);
                else
                    state <= S_TNSD;
            end
            S_TNSR: begin
                tres <= gb_acc[0];
                getbits(short_w ? 5'd7 : 5'd11, S_TNSF);
            end
            S_TNSF: begin
                // length, order
                if (short_w) begin
                    tlen <= {2'd0, gb_acc[6:3]};
                    tord <= {2'd0, gb_acc[2:0]};
                end else begin
                    tlen <= gb_acc[10:5];
                    tord <= gb_acc[4:0];
                end
                state <= S_TNSF2;
            end
            S_TNSF2: begin
                ti <= 5'd0;
                if (tord != 5'd0)
                    getbits(5'd2, S_TNSF3);
                else
                    state <= S_TNSF4;
            end
            S_TNSF3: begin
                tdir <= gb_acc[1];
                tcomp <= gb_acc[0];
                state <= S_TNSC;
            end
            S_TNSC: begin
                if (ti == tord)
                    state <= S_TNSF4;
                else
                    getbits({3'd0, 2'd0} + (tres ? 5'd4 : 5'd3) - {4'd0, tcomp}, S_TNSC2);
            end
            S_TNSC2: begin
                // Keep the first TNS_MAX_ORDER coefficients, as an index
                // into the reflection coefficient table.
                if (ti < (short_w ? 5'd7 : 5'd12)) begin
                    tc_we   <= 1'b1;
                    tc_ch   <= ch;
                    tc_addr <= {tcount[2:0], ti[3:0]};
                    tc_data <= {tres, tns_sext(gb_acc[3:0],
                                               (tres ? 3'd4 : 3'd3) - {2'd0, tcomp})};
                end
                ti <= ti + 5'd1;
                state <= S_TNSC;
            end
            S_TNSF4: begin
                // A filter of this window is done: its band range.
                // top = bottom; bottom = max(top - length, 0)
                begin : tnsrange
                    reg [5:0] top, bot, lim, lo, hi;
                    reg [3:0] ord;
                    top = tbot;
                    bot = (tlen > tbot) ? 6'd0 : tbot - tlen;
                    lim = (tns_max < msfb) ? tns_max : msfb;
                    lo  = (bot < lim) ? bot : lim;
                    hi  = (top < lim) ? top : lim;
                    ord = (tord > (short_w ? 5'd7 : 5'd12)) ?
                          (short_w ? 4'd7 : 4'd12) : tord[3:0];
                    tbot <= bot;
                    if (ord != 4'd0 && hi > lo) begin
                        td_we   <= 1'b1;
                        td_ch   <= ch;
                        td_idx  <= tcount[2:0];
                        td_data <= {tw, lo, hi, tdir, ord};
                        tcount  <= tcount + 4'd1;
                    end
                end
                if (tf + 2'd1 == tnf)
                    state <= S_TNSD;
                else begin
                    tf <= tf + 2'd1;
                    getbits(short_w ? 5'd7 : 5'd11, S_TNSF);
                end
            end
            S_TNSD: begin
                // Next window.
                if ({1'b0, tw} + 4'd1 == nwin)
                    getbits(5'd1, S_GAIN);
                else begin
                    tw <= tw + 3'd1;
                    state <= S_TNSW;
                end
            end
            S_GAIN: begin
                if (ch) ntns1 <= tcount; else ntns0 <= tcount;
                if (gb_acc[0])
                    fail(4'd4);         // gain_control_data_present
                else begin
                    g <= 3'd0; w0 <= 4'd0; glen <= glen_f(grp, 4'd0, short_w);
                    band_a <= 6'd0;
                    state <= S_SP;
                end
            end

            // ----------------------------------------------------------
            // spectral_data, every coefficient out once
            S_SP: begin
                if (w0 == nwin) begin
                    win <= 4'd0;
                    band_a <= msfb;
                    state <= S_SPTW;
                end else if (band_a == msfb) begin
                    g <= g + 3'd1;
                    w0 <= w0 + glen;
                    glen <= glen_f(grp, w0 + glen, short_w);
                    band_a <= 6'd0;
                end else begin
                    lb_raddr <= baddr;
                    state <= S_SPB;
                end
            end
            S_SPB: state <= S_SPB2;
            S_SPB2: begin
                cur_cb <= lb_q[11:8];
                cur_sf <= lb_q[7:0];
                width  <= off_b[6:0] - off_a[6:0];
                win    <= 4'd0;
                state  <= S_SPW;
            end
            S_SPW: begin
                if (win == glen) begin
                    band_a <= band_a + 6'd1;
                    state <= S_SP;
                end else begin
                    base <= short_w ? {w0[2:0] + win[2:0], 7'd0} + off_a : off_a;
                    bin <= 7'd0;
                    d <= 2'd0;
                    dim <= cb_quad ? 3'd4 : 3'd2;
                    if (cb_none)
                        state <= S_SPZ;
                    else
                        state <= S_SPC;
                end
            end
            S_SPZ: begin
                c_valid <= 1'b1; c_ch <= ch; c_addr <= emit_addr[9:0];
                c_q <= 15'd0; c_sf <= cur_sf; c_zero <= 1'b1;
                if (bin + 7'd1 == width) begin
                    win <= win + 4'd1;
                    state <= S_SPW;
                end else
                    bin <= bin + 7'd1;
            end
            S_SPC: begin
                huff(cur_cb);
                state <= S_SPCW;
            end
            S_SPCW: begin
                if (h_busy && !h_start && h_hit) begin
                    h_busy <= 1'b0;
                    d <= 2'd0;
                    if (cb_quad) begin
                        if (cb_unsigned) begin
                            v0 <= {13'd0, h_pay[7:6]};
                            v1 <= {13'd0, h_pay[5:4]};
                            v2 <= {13'd0, h_pay[3:2]};
                            v3 <= {13'd0, h_pay[1:0]};
                        end else begin
                            v0 <= {{13{h_pay[7]}}, h_pay[7:6]};
                            v1 <= {{13{h_pay[5]}}, h_pay[5:4]};
                            v2 <= {{13{h_pay[3]}}, h_pay[3:2]};
                            v3 <= {{13{h_pay[1]}}, h_pay[1:0]};
                        end
                    end else begin
                        if (cb_unsigned) begin
                            v0 <= {10'd0, h_pay[9:5]};
                            v1 <= {10'd0, h_pay[4:0]};
                        end else begin
                            v0 <= {{10{h_pay[9]}}, h_pay[9:5]};
                            v1 <= {{10{h_pay[4]}}, h_pay[4:0]};
                        end
                        v2 <= 15'd0;
                        v3 <= 15'd0;
                    end
                    state <= cb_unsigned ? S_SPS : S_SPO;
                end else if (in_rdb && at_end && bit_avail)
                    fail(cfg_raw ? 4'd10 : 4'd9);
            end
            S_SPS: begin
                // One sign bit for each nonzero value, lowest first.
                if (vd == 15'd0) begin
                    if ({1'b0, d} == dim - 3'd1) begin
                        d <= 2'd0;
                        state <= (cur_cb == 4'd11) ? S_SPE : S_SPO;
                    end else
                        d <= d + 2'd1;
                end else if (bit_avail && in_rdb && at_end) begin
                    fail(cfg_raw ? 4'd10 : 4'd9);
                end else if (can_take) begin
                    if (bit_val) begin
                        case (d)
                            2'd0: v0 <= 15'd0 - v0;
                            2'd1: v1 <= 15'd0 - v1;
                            2'd2: v2 <= 15'd0 - v2;
                            default: v3 <= 15'd0 - v3;
                        endcase
                    end
                    if ({1'b0, d} == dim - 3'd1) begin
                        d <= 2'd0;
                        state <= (cur_cb == 4'd11) ? S_SPE : S_SPO;
                    end else
                        d <= d + 2'd1;
                end
            end
            S_SPE: begin
                // Escapes, y then z (4.6.3.3): |v| == 16 is the flag.
                if (d == 2'd2) begin
                    d <= 2'd0;
                    state <= S_SPO;
                end else if (vd == 15'd16 || vd == 15'h7ff0) begin
                    esc_n <= 4'd0;
                    state <= S_SPEP;
                end else
                    d <= d + 2'd1;
            end
            S_SPEP: begin
                if (bit_avail && in_rdb && at_end)
                    fail(cfg_raw ? 4'd10 : 4'd9);
                else if (can_take) begin
                    if (bit_val) begin
                        if (esc_n == 4'd8)
                            fail(4'd8);
                        else
                            esc_n <= esc_n + 4'd1;
                    end else
                        getbits({1'b0, esc_n} + 5'd4, S_SPEW);
                end
            end
            S_SPEW: begin
                begin : escval
                    reg [14:0] mag;
                    mag = (15'd1 << (esc_n + 4'd4)) | {2'd0, gb_acc[12:0]};
                    if (d == 2'd0)
                        v0 <= v0[14] ? 15'd0 - mag : mag;
                    else
                        v1 <= v1[14] ? 15'd0 - mag : mag;
                end
                d <= d + 2'd1;
                state <= S_SPE;
            end
            S_SPO: begin
                // Out, one a cycle.
                c_valid <= 1'b1; c_ch <= ch; c_addr <= emit_addr[9:0];
                c_q <= pq3; c_sf <= cur_sf; c_zero <= 1'b0;
                if ({1'b0, d} == dim - 3'd1) begin
                    d <= 2'd0;
                    if (bin + {4'd0, dim} == width) begin
                        win <= win + 4'd1;
                        state <= S_SPW;
                    end else begin
                        bin <= bin + {4'd0, dim};
                        state <= S_SPC;
                    end
                end else
                    d <= d + 2'd1;
            end
            S_SPTW: state <= S_SPT;      // the band table's read
            S_SPT: begin
                // Everything from max_sfb up, in every window, is zero.
                if (win == nwin)
                    state <= S_ICSEND;
                else begin
                    tail_k <= off_a;
                    state <= S_SPT2;
                end
            end
            S_SPT2: begin
                if (tail_k == (short_w ? 11'd128 : 11'd1024)) begin
                    win <= win + 4'd1;
                    state <= S_SPT;
                end else begin
                    c_valid <= 1'b1; c_ch <= ch;
                    c_addr <= short_w ? {win[2:0], tail_k[6:0]} : tail_k[9:0];
                    c_q <= 15'd0; c_sf <= 8'd0; c_zero <= 1'b1;
                    tail_k <= tail_k + 11'd1;
                end
            end
            S_ICSEND: begin
                if (cpe && !ch) begin
                    ch <= 1'b1;
                    getbits(5'd8, S_ICS);
                end else
                    state <= S_ID;
            end
            default: state <= S_IDLE;
            endcase

            // A Huffman read that ran into the end of the data.
            if (h_busy && !h_start && !h_hit && in_rdb && at_end && bit_avail &&
                (state == S_SF4 || state == S_SPCW)) begin
                h_busy <= 1'b0;
            end
        end
    end
endmodule
