// aac_dec -- an AAC-LC decoder: ADTS or raw access units in, 16-bit PCM out.
//
// MPEG-4 AAC Low Complexity (ISO/IEC 14496-3 subpart 4) and MPEG-2 AAC LC
// (ISO/IEC 13818-7), mono or stereo, 8 to 48 kHz, 1024-sample frames, every
// coding tool of the profile: section and scalefactor data, the eleven
// spectrum codebooks and escapes, pulse data, M/S and intensity stereo,
// perceptual noise substitution, TNS, long, start, stop and eight-short
// window sequences with sine and KBD windows, overlap-add.
//
// Input: bytes, valid/ready. With cfg_raw low they are an ADTS stream
// (1.A.3), resynchronised on the syncword after anything it refuses. With
// cfg_raw high each access unit is one raw_data_block(), as MP4 and
// LATM-free MPEG-TS payloads carry it, with in_last on its last byte; the
// sampling frequency index and channel count come from cfg_sfi and
// cfg_chan, which an AudioSpecificConfig holds (samplingFrequencyIndex,
// channelConfiguration).
//
// Output: 16-bit samples, valid/ready, interleaved left then right for
// stereo, out_last on the 1024th sample of a frame's last channel. A frame
// the decoder refuses produces no samples and a frame_err pulse with
// err_code (README, "Errors").
//
// The frame's stages run one after another; only the output overlaps the
// next frame. README, "Cycles", has what a frame costs against the 48 kHz
// budget.

module aac_dec (
    input  wire        clk,
    input  wire        rst_n,

    input  wire        cfg_raw,
    input  wire [3:0]  cfg_sfi,
    input  wire [1:0]  cfg_chan,

    input  wire [7:0]  in_data,
    input  wire        in_valid,
    input  wire        in_last,
    output wire        in_ready,

    output wire [15:0] out_data,
    output wire        out_right,
    output wire        out_last,
    output wire        out_valid,
    input  wire        out_ready,

    output reg  [3:0]  stat_sfi,
    output reg         stat_stereo,
    output reg         frame_ok,
    output reg         frame_err,
    output reg  [3:0]  err_code
);
    // ------------------------------------------------------------------
    // Bits and syntax
    // ------------------------------------------------------------------
    wire bit_avail, bit_val, bit_eob, bit_last, bit_take;
    aac_bits u_bits (
        .clk(clk), .rst_n(rst_n), .in_data(in_data), .in_valid(in_valid),
        .in_last(in_last), .in_ready(in_ready), .bit_avail(bit_avail),
        .bit_val(bit_val), .bit_eob(bit_eob), .bit_last(bit_last),
        .bit_take(bit_take)
    );

    reg  go;
    wire blk_done, blk_err, p_idle, rdb_wait;
    wire [3:0] p_err, sfi;
    wire cpe, common;
    wire [1:0] ms_mask, ws0, ws1;
    wire shape0, shape1;
    wire [5:0] msfb0, msfb1;
    wire [6:0] grp0, grp1;
    wire [3:0] ntns0, ntns1;
    wire band_we, band_ch, ms_we, ms_bit, td_we, td_ch, tc_we, tc_ch;
    wire [6:0] band_addr, ms_addr, tc_addr;
    wire [3:0] band_cb;
    wire [11:0] band_val;
    wire [2:0] td_idx;
    wire [19:0] td_data;
    wire [4:0] tc_data;
    wire c_valid, c_ch, c_zero;
    wire [9:0] c_addr;
    wire [14:0] c_q;
    wire [7:0] c_sf;
    aac_parse u_parse (
        .clk(clk), .rst_n(rst_n), .cfg_raw(cfg_raw), .cfg_sfi(cfg_sfi),
        .cfg_chan(cfg_chan), .bit_avail(bit_avail), .bit_val(bit_val),
        .bit_eob(bit_eob), .bit_last(bit_last), .bit_take(bit_take),
        .go(go), .blk_done(blk_done), .blk_err(blk_err), .err_code(p_err),
        .idle(p_idle), .rdb_wait(rdb_wait), .sfi(sfi), .cpe(cpe), .common(common),
        .ms_mask(ms_mask), .ws0(ws0), .ws1(ws1), .shape0(shape0), .shape1(shape1),
        .msfb0(msfb0), .msfb1(msfb1), .grp0(grp0), .grp1(grp1),
        .ntns0(ntns0), .ntns1(ntns1),
        .band_we(band_we), .band_ch(band_ch), .band_addr(band_addr),
        .band_cb(band_cb), .band_val(band_val),
        .ms_we(ms_we), .ms_addr(ms_addr), .ms_bit(ms_bit),
        .td_we(td_we), .td_ch(td_ch), .td_idx(td_idx), .td_data(td_data),
        .tc_we(tc_we), .tc_ch(tc_ch), .tc_addr(tc_addr), .tc_data(tc_data),
        .c_valid(c_valid), .c_ch(c_ch), .c_addr(c_addr), .c_q(c_q),
        .c_sf(c_sf), .c_zero(c_zero)
    );

    wire w_valid, w_ch;
    wire [9:0] w_addr;
    wire [34:0] w_data;
    aac_dequant u_dq (
        .clk(clk), .rst_n(rst_n), .c_valid(c_valid), .c_ch(c_ch),
        .c_addr(c_addr), .c_q(c_q), .c_sf(c_sf), .c_zero(c_zero),
        .w_valid(w_valid), .w_ch(w_ch), .w_addr(w_addr), .w_data(w_data)
    );

    // ------------------------------------------------------------------
    // The sequencer
    // ------------------------------------------------------------------
    localparam [3:0]
        Q_IDLE = 4'd0, Q_PARSE = 4'd1, Q_DRAIN = 4'd2, Q_STEREO = 4'd3,
        Q_TNS = 4'd4, Q_WAITBUF = 4'd5, Q_SYNTH = 4'd6, Q_COMMIT = 4'd7,
        Q_START = 4'd8;
    reg [3:0] q;
    reg       qch;         // channel in TNS and synthesis
    reg [3:0] qdrain;
    reg       first;
    reg       st_start, tns_start, syn_start;
    wire      st_done, tns_done, syn_done;
    wire      can_write;
    reg       commit;

    // ------------------------------------------------------------------
    // The two spectra, and who drives them
    // ------------------------------------------------------------------
    wire        st_l_we, st_r_we;
    wire [9:0]  st_l_waddr, st_r_waddr, st_l_raddr, st_r_raddr;
    wire [34:0] st_l_wdata, st_r_wdata;
    wire [9:0]  tn_raddr, tn_waddr;
    wire        tn_we;
    wire [34:0] tn_wdata;
    wire [9:0]  sy_raddr_a, sy_raddr_b;

    reg         l_we, r_we;
    reg  [9:0]  l_waddr, r_waddr, l_raddr_a, r_raddr_a, l_raddr_b, r_raddr_b;
    reg  [34:0] l_wdata, r_wdata;
    wire [34:0] l_q_a, l_q_b, r_q_a, r_q_b;

    always @(*) begin
        l_we = 1'b0; l_waddr = 10'd0; l_wdata = 35'd0;
        r_we = 1'b0; r_waddr = 10'd0; r_wdata = 35'd0;
        l_raddr_a = 10'd0; l_raddr_b = 10'd0;
        r_raddr_a = 10'd0; r_raddr_b = 10'd0;
        case (q)
        Q_PARSE, Q_DRAIN: begin
            l_we = w_valid && !w_ch; l_waddr = w_addr; l_wdata = w_data;
            r_we = w_valid && w_ch;  r_waddr = w_addr; r_wdata = w_data;
        end
        Q_STEREO: begin
            l_we = st_l_we; l_waddr = st_l_waddr; l_wdata = st_l_wdata;
            r_we = st_r_we; r_waddr = st_r_waddr; r_wdata = st_r_wdata;
            l_raddr_a = st_l_raddr;
            r_raddr_a = st_r_raddr;
        end
        Q_TNS: begin
            if (qch) begin
                r_we = tn_we; r_waddr = tn_waddr; r_wdata = tn_wdata;
                r_raddr_a = tn_raddr;
            end else begin
                l_we = tn_we; l_waddr = tn_waddr; l_wdata = tn_wdata;
                l_raddr_a = tn_raddr;
            end
        end
        default: begin
            l_raddr_a = sy_raddr_a; l_raddr_b = sy_raddr_b;
            r_raddr_a = sy_raddr_a; r_raddr_b = sy_raddr_b;
        end
        endcase
    end

    aac_ram #(.WIDTH(35), .DEPTH(1024), .AW(10)) u_spec_l (
        .clk(clk), .we(l_we), .waddr(l_waddr), .wdata(l_wdata),
        .raddr_a(l_raddr_a), .rdata_a(l_q_a), .raddr_b(l_raddr_b), .rdata_b(l_q_b)
    );
    aac_ram #(.WIDTH(35), .DEPTH(1024), .AW(10)) u_spec_r (
        .clk(clk), .we(r_we), .waddr(r_waddr), .wdata(r_wdata),
        .raddr_a(r_raddr_a), .rdata_a(r_q_a), .raddr_b(r_raddr_b), .rdata_b(r_q_b)
    );

    // ------------------------------------------------------------------
    // The stages
    // ------------------------------------------------------------------
    aac_stereo u_stereo (
        .clk(clk), .rst_n(rst_n), .start(st_start), .done(st_done),
        .sfi(sfi), .cpe(cpe), .common(common), .ms_mask(ms_mask),
        .ws0(ws0), .ws1(ws1), .msfb0(msfb0), .msfb1(msfb1),
        .grp0(grp0), .grp1(grp1),
        .band_we(band_we), .band_ch(band_ch), .band_addr(band_addr),
        .band_cb(band_cb), .band_val(band_val),
        .ms_we(ms_we), .ms_addr(ms_addr), .ms_bit(ms_bit),
        .l_we(st_l_we), .l_waddr(st_l_waddr), .l_wdata(st_l_wdata),
        .l_raddr(st_l_raddr), .l_rdata(l_q_a),
        .r_we(st_r_we), .r_waddr(st_r_waddr), .r_wdata(st_r_wdata),
        .r_raddr(st_r_raddr), .r_rdata(r_q_a)
    );

    aac_tns u_tns (
        .clk(clk), .rst_n(rst_n), .start(tns_start), .ch(qch), .done(tns_done),
        .sfi(sfi), .ws(qch ? ws1 : ws0), .ntns(qch ? ntns1 : ntns0),
        .td_we(td_we), .td_ch(td_ch), .td_idx(td_idx), .td_data(td_data),
        .tc_we(tc_we), .tc_ch(tc_ch), .tc_addr(tc_addr), .tc_data(tc_data),
        .raddr(tn_raddr), .rdata(qch ? r_q_a : l_q_a),
        .we(tn_we), .waddr(tn_waddr), .wdata(tn_wdata)
    );

    wire pcm_we, pcm_ch;
    wire [9:0] pcm_n;
    wire [15:0] pcm_data;
    aac_synth u_synth (
        .clk(clk), .rst_n(rst_n), .start(syn_start), .ch(qch),
        .ws(qch ? ws1 : ws0), .shape(qch ? shape1 : shape0), .first(first),
        .done(syn_done),
        .sp_raddr_a(sy_raddr_a), .sp_raddr_b(sy_raddr_b),
        .sp_rdata_a(qch ? r_q_a : l_q_a), .sp_rdata_b(qch ? r_q_b : l_q_b),
        .pcm_we(pcm_we), .pcm_ch(pcm_ch), .pcm_n(pcm_n), .pcm_data(pcm_data)
    );

    aac_pcm_out u_out (
        .clk(clk), .rst_n(rst_n), .pcm_we(pcm_we), .pcm_ch(pcm_ch),
        .pcm_n(pcm_n), .pcm_data(pcm_data), .commit(commit), .stereo(cpe),
        .can_write(can_write), .out_data(out_data), .out_right(out_right),
        .out_last(out_last), .out_valid(out_valid), .out_ready(out_ready)
    );

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            q <= Q_IDLE; qch <= 1'b0; qdrain <= 4'd0; first <= 1'b1; go <= 1'b1;
            st_start <= 1'b0; tns_start <= 1'b0; syn_start <= 1'b0; commit <= 1'b0;
            stat_sfi <= 4'd0; stat_stereo <= 1'b0; frame_ok <= 1'b0;
            frame_err <= 1'b0; err_code <= 4'd0;
        end else begin
            st_start <= 1'b0;
            tns_start <= 1'b0;
            syn_start <= 1'b0;
            commit <= 1'b0;
            frame_ok <= 1'b0;
            frame_err <= 1'b0;
            if (blk_err) begin
                frame_err <= 1'b1;
                err_code <= p_err;
            end
            case (q)
            Q_IDLE: begin
                go <= 1'b1;
                if (go && rdb_wait) begin
                    go <= 1'b0;
                    q <= Q_PARSE;
                end
            end
            Q_PARSE: begin
                if (blk_err) begin
                    go <= 1'b1;
                    q <= Q_IDLE;
                end else if (blk_done) begin
                    qdrain <= 4'd8;
                    q <= Q_DRAIN;
                end
            end
            Q_DRAIN: begin
                // the last coefficients through aac_dequant
                if (qdrain == 4'd0) begin
                    st_start <= 1'b1;
                    q <= Q_STEREO;
                end else
                    qdrain <= qdrain - 4'd1;
            end
            Q_STEREO: begin
                if (st_done) begin
                    qch <= 1'b0;
                    q <= Q_START;
                end
            end
            Q_START: begin
                tns_start <= 1'b1;
                q <= Q_TNS;
            end
            Q_TNS: begin
                if (tns_done)
                    q <= Q_WAITBUF;
            end
            Q_WAITBUF: begin
                if (can_write) begin
                    syn_start <= 1'b1;
                    q <= Q_SYNTH;
                end
            end
            Q_SYNTH: begin
                if (syn_done) begin
                    if (cpe && !qch) begin
                        qch <= 1'b1;
                        q <= Q_START;
                    end else
                        q <= Q_COMMIT;
                end
            end
            Q_COMMIT: begin
                commit <= 1'b1;
                first <= 1'b0;
                frame_ok <= 1'b1;
                stat_sfi <= sfi;
                stat_stereo <= cpe;
                go <= 1'b1;
                q <= Q_IDLE;
            end
            default: q <= Q_IDLE;
            endcase
        end
    end
endmodule
