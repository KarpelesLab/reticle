// aac_stereo -- perceptual noise substitution, M/S and intensity stereo.
//
// One pass over the bands of a raw_data_block, after every coefficient is
// in the spectral RAMs and before TNS (4.6.13.5: noise goes in before TNS).
// For a CPE with a common window, band by band and in this order:
//
//   1. the left band, if it is noise, is filled with +-V,
//      V = 2^(nrg/4) / sqrt(width): a band of +-1s has exactly the energy
//      width, so V scales it to the transmitted energy 2^(nrg/2) without
//      the square root and division 4.6.13.3 computes. The signs come from
//      a PRBS31 generator, which runs on across frames.
//   2. the right band likewise; if both are noise and ms_used is set the
//      right one replays the left one's signs (4.6.13.3, correlated noise).
//   3. intensity (right codebook 14 or 15): r = +-2^(-pos/4) * l, the sign
//      flipped by ms_used when ms_mask_present is 1 (4.6.8.2.3);
//      or M/S, l' = l + r and r' = l - r, where ms_used says and neither
//      band is noise (4.6.8.1.3).
//
// An SCE, or a CPE without a common window, gets step 1 on each channel.
// `tools/aac_dec_model.py` (`FixedDecoder.stereo`) is the arithmetic.

module aac_stereo (
    input  wire        clk,
    input  wire        rst_n,
    input  wire        start,
    output reg         done,

    input  wire [3:0]  sfi,
    input  wire        cpe,
    input  wire        common,
    input  wire [1:0]  ms_mask,
    input  wire [1:0]  ws0,
    input  wire [1:0]  ws1,
    input  wire [5:0]  msfb0,
    input  wire [5:0]  msfb1,
    input  wire [6:0]  grp0,
    input  wire [6:0]  grp1,

    input  wire        band_we,
    input  wire        band_ch,
    input  wire [6:0]  band_addr,
    input  wire [3:0]  band_cb,
    input  wire [11:0] band_val,
    input  wire        ms_we,
    input  wire [6:0]  ms_addr,
    input  wire        ms_bit,

    output reg         l_we,
    output reg  [9:0]  l_waddr,
    output reg  [34:0] l_wdata,
    output reg  [9:0]  l_raddr,
    input  wire [34:0] l_rdata,
    output reg         r_we,
    output reg  [9:0]  r_waddr,
    output reg  [34:0] r_wdata,
    output reg  [9:0]  r_raddr,
    input  wire [34:0] r_rdata
);
    // ------------------------------------------------------------------
    // Band information, written by the parser
    // ------------------------------------------------------------------
    reg [15:0] bl_mem [0:127];
    reg [15:0] br_mem [0:127];
    reg        ms_mem [0:127];
    reg [6:0]  b_raddr;
    reg [15:0] bl_q, br_q;
    reg        ms_q;
    always @(posedge clk) begin
        if (band_we && !band_ch) bl_mem[band_addr] <= {band_cb, band_val};
        if (band_we && band_ch)  br_mem[band_addr] <= {band_cb, band_val};
        if (ms_we)               ms_mem[ms_addr]   <= ms_bit;
        bl_q <= bl_mem[b_raddr];
        br_q <= br_mem[b_raddr];
        ms_q <= ms_mem[b_raddr];
    end

    // ------------------------------------------------------------------
    // Iteration over (group, window, band) of channel `c`
    // ------------------------------------------------------------------
    localparam [3:0]
        T_IDLE = 4'd0, T_BAND = 4'd1, T_READ = 4'd2, T_LATCH = 4'd3,
        T_STEP = 4'd4, T_VMUL = 4'd5, T_VMUL2 = 4'd6, T_NOISE = 4'd7,
        T_LOOP = 4'd8, T_DRAIN = 4'd9;
    reg [3:0]  st;
    reg        joint;      // a CPE with a common window
    reg        c;          // channel, when not joint
    reg [2:0]  g;
    reg [3:0]  w0;
    reg [3:0]  win;
    reg [5:0]  sfb;
    reg [1:0]  step;       // 0 noise left, 1 noise right, 2 intensity or M/S

    wire [1:0] ws   = c ? ws1 : ws0;
    wire [5:0] msfb = c ? msfb1 : msfb0;
    wire [6:0] grp  = c ? grp1 : grp0;
    wire       short_w = ws == 2'd2;
    wire [3:0] nwin = short_w ? 4'd8 : 4'd1;
    wire [3:0] glen;
    aac_glen u_glen (.grp(grp), .w0(w0), .short_win(short_w), .glen(glen));

    // The offsets of sfb and sfb + 1, a cycle after sfb is set: sfb is
    // two edges old by T_LATCH, where they are used.
    wire [10:0] off_a, off_b;
    wire [5:0]  nsw, tmx;
    aac_swb u_swb (.clk(clk), .sfi(sfi), .short_win(short_w),
                   .band_a(sfb), .band_b(sfb + 6'd1),
                   .off_a(off_a), .off_b(off_b),
                   .num_swb(nsw), .tns_max(tmx));

    // The band being processed.
    reg [3:0]  cb_l, cb_r;
    reg [11:0] val_l, val_r;
    reg        msu;
    reg [9:0]  base;
    reg [6:0]  width;
    reg [6:0]  i;          // coefficients issued
    reg [3:0]  pend;       // cycles of drain left

    wire ms_on   = ms_mask == 2'd2 || (ms_mask == 2'd1 && msu);
    wire noise_l = cb_l == 4'd13;
    wire noise_r = joint && cb_r == 4'd13;
    wire is_r    = joint && (cb_r == 4'd14 || cb_r == 4'd15);
    wire do_ms   = joint && ms_on && !is_r && !noise_l && cb_r != 4'd13;

    // ------------------------------------------------------------------
    // The multiplier: 35 x 25 signed, registered in and out
    // ------------------------------------------------------------------
    reg  signed [34:0] m_a;
    reg  signed [24:0] m_b;
    reg  signed [59:0] m_p;
    // The product registered twice: once in the DSP slices, and once more
    // after the adder that joins their two halves, which is fabric.
    reg  signed [59:0] m_q;
    always @(posedge clk) begin
        m_p <= m_a * m_b;
        m_q <= m_p;
    end

    wire [24:0] gm;
    reg  [1:0]  gm_idx;
    aac_gain_mant u_gm (.idx(gm_idx), .q(gm));
    wire [24:0] isq;
    aac_inv_sqrt u_isq (.idx(width[6:2] - 5'd1), .q(isq));

    // PNS: V = (gm * isq) >> (39 - e), e = nrg >> 2, saturated: the shift,
    // its rounding half and the out-of-range cases are fixed for the band
    // and set up before the product; the shift and the saturation are a
    // cycle each.
    reg  [5:0]  vsh;       // 39 - e, where it is 9..47
    reg  [47:0] vhalf;
    reg         vsat;      // 39 - e <= 8: the value saturates
    reg         vzero;     // 39 - e >= 48: the value is zero
    reg  [47:0] vsh_r;
    reg  [34:0] vval;
    wire [47:0] vshift = ({1'b0, m_q[46:0]} + vhalf) >> vsh;
    wire [34:0] vnext = vsat ? 35'h3_FFFF_FFFF : vzero ? 35'd0 :
                        (vsh_r > 48'h3_FFFF_FFFF ? 35'h3_FFFF_FFFF : vsh_r[34:0]);

    // Intensity: r = (l * gm) >> (23 - (gexp >> 2)), gexp = -pos. Likewise
    // the shift (offset by ten, 0..69) and its half are the band's; p4 shifts
    // and p5 saturates and signs.
    reg  [6:0]  is_s;
    reg  signed [69:0] ihalf;
    reg         izero;     // the shift is 60 or more: zero
    reg         isatsh;    // the shift is -11 or less: saturate unless zero
    reg         ineg;
    wire signed [69:0] ipe = {m_q, 10'd0};
    wire signed [69:0] ishf = (ipe + ihalf) >>> is_s;
    wire signed [34:0] imax = 35'sh3_FFFF_FFFF;
    wire signed [34:0] imin = -35'sh4_0000_0000;
    reg  [34:0] i_lo;      // p5: the shift's low bits, whether they overflow,
    reg         i_ovf;     //     its sign, and the product's sign and zero
    reg         i_sign;
    reg         i_pzero;
    reg         i_psign;
    wire signed [34:0] ival = izero ? 35'sd0 :
                              isatsh ? (i_pzero ? 35'sd0 : (i_psign ? imin : imax)) :
                              i_ovf ? (i_sign ? imin : imax) : $signed(i_lo);
    wire signed [34:0] ineg_v = (ival == imin) ? imax : -ival;

    // The noise value's shift for a band of energy nrg: 39 - (nrg >> 2).
    task vprep;
        input [11:0] nrg;
        reg   [10:0] sh;
        begin
            sh = 11'd39 - {nrg[11], nrg[11:2]};
            vsh <= sh[5:0];
            vhalf <= (sh[10] || sh == 11'd0) ? 48'd0 : (48'd1 << (sh[5:0] - 6'd1));
            vsat <= sh[10] || sh <= 11'd8;
            vzero <= !sh[10] && sh >= 11'd48;
        end
    endtask

    // M/S sums, saturated.
    wire signed [35:0] msum = {l_rdata[34], l_rdata} + {r_rdata[34], r_rdata};
    wire signed [35:0] mdif = {l_rdata[34], l_rdata} - {r_rdata[34], r_rdata};
    function [34:0] sat36;
        input [35:0] v;
        begin
            if (v[35] != v[34])
                sat36 = v[35] ? 35'h4_0000_0000 : 35'h3_FFFF_FFFF;
            else
                sat36 = v[34:0];
        end
    endfunction

    // The PNS sign generator.
    reg  [30:0] prbs;
    reg  [30:0] prbs_save;
    wire        prbs_bit = prbs[30] ^ prbs[27];

    // Pipeline of the per-coefficient loops: what was read where.
    reg  [9:0]  p1_addr, p2_addr, p3_addr, p4_addr, p5_addr, p6_addr;
    reg         p1_v, p2_v, p3_v, p4_v, p5_v, p6_v;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            st <= T_IDLE; done <= 1'b0; joint <= 1'b0; c <= 1'b0;
            g <= 3'd0; w0 <= 4'd0; win <= 4'd0; sfb <= 6'd0; step <= 2'd0;
            b_raddr <= 7'd0;
            cb_l <= 4'd0; cb_r <= 4'd0; val_l <= 12'd0; val_r <= 12'd0; msu <= 1'b0;
            base <= 10'd0; width <= 7'd0; i <= 7'd0; pend <= 4'd0;
            m_a <= 35'sd0; m_b <= 25'sd0; gm_idx <= 2'd0;
            vval <= 35'd0; ineg <= 1'b0;
            prbs <= 31'd1; prbs_save <= 31'd1;
            p1_addr <= 10'd0; p2_addr <= 10'd0; p3_addr <= 10'd0; p4_addr <= 10'd0;
            p5_addr <= 10'd0; p6_addr <= 10'd0;
            p1_v <= 1'b0; p2_v <= 1'b0; p3_v <= 1'b0; p4_v <= 1'b0; p5_v <= 1'b0;
            p6_v <= 1'b0;
            vsh <= 6'd0; vhalf <= 48'd0; vsat <= 1'b0; vzero <= 1'b0; vsh_r <= 48'd0;
            is_s <= 7'd0; ihalf <= 70'sd0; izero <= 1'b0; isatsh <= 1'b0;
            i_lo <= 35'd0; i_ovf <= 1'b0; i_sign <= 1'b0; i_pzero <= 1'b0; i_psign <= 1'b0;
            l_we <= 1'b0; l_waddr <= 10'd0; l_wdata <= 35'd0; l_raddr <= 10'd0;
            r_we <= 1'b0; r_waddr <= 10'd0; r_wdata <= 35'd0; r_raddr <= 10'd0;
        end else begin
            done <= 1'b0;
            l_we <= 1'b0;
            r_we <= 1'b0;
            case (st)
            T_IDLE: begin
                if (start) begin
                    joint <= cpe && common;
                    c <= 1'b0;
                    g <= 3'd0; w0 <= 4'd0; win <= 4'd0; sfb <= 6'd0;
                    st <= T_BAND;
                end
            end
            T_BAND: begin
                if (w0 == nwin) begin
                    if (!joint && cpe && !c) begin
                        c <= 1'b1;
                        g <= 3'd0; w0 <= 4'd0; win <= 4'd0; sfb <= 6'd0;
                    end else begin
                        done <= 1'b1;
                        st <= T_IDLE;
                    end
                end else if (win == glen) begin
                    g <= g + 3'd1;
                    w0 <= w0 + glen;
                    win <= 4'd0;
                    sfb <= 6'd0;
                end else if (sfb == msfb) begin
                    win <= win + 4'd1;
                    sfb <= 6'd0;
                end else begin
                    b_raddr <= short_w ? {g, sfb[3:0]} : {1'b0, sfb};
                    st <= T_READ;
                end
            end
            T_READ: st <= T_LATCH;
            T_LATCH: begin
                // In a separate pass the channel's own band RAM is "left".
                cb_l  <= (!joint && c) ? br_q[15:12] : bl_q[15:12];
                val_l <= (!joint && c) ? br_q[11:0]  : bl_q[11:0];
                cb_r  <= br_q[15:12];
                val_r <= br_q[11:0];
                msu   <= ms_q;
                width <= off_b[6:0] - off_a[6:0];
                base  <= short_w ? {w0[2:0] + win[2:0], 7'd0} + off_a[9:0] : off_a[9:0];
                step  <= 2'd0;
                st    <= T_STEP;
            end
            T_STEP: begin
                i <= 7'd0;
                if (step == 2'd0 && noise_l) begin
                    // V for the left (or only) channel.
                    gm_idx <= val_l[1:0];
                    vprep(val_l);
                    prbs_save <= prbs;
                    st <= T_VMUL;
                end else if (step == 2'd1 && noise_r) begin
                    gm_idx <= val_r[1:0];
                    vprep(val_r);
                    if (noise_l && ms_on)
                        prbs <= prbs_save;
                    st <= T_VMUL;
                end else if (step == 2'd2 && (is_r || do_ms)) begin
                    // gexp = -pos; ish = 23 - (gexp >> 2)
                    begin : isprep
                        reg [12:0] gexp;
                        reg [10:0] sh;    // 23 - (gexp >> 2), signed
                        reg [6:0]  s10;
                        gexp = 13'd0 - {val_r[11], val_r};
                        sh = 11'd23 - gexp[12:2];
                        s10 = sh[6:0] + 7'd10;
                        gm_idx <= gexp[1:0];
                        is_s <= s10;
                        ihalf <= (s10 == 7'd0) ? 70'sd0 : (70'sd1 <<< (s10 - 7'd1));
                        izero <= !sh[10] && sh >= 11'd60;
                        isatsh <= $signed(sh) <= -11'sd11;
                        ineg <= (cb_r == 4'd14) != (ms_mask == 2'd1 && msu);
                    end
                    p1_v <= 1'b0; p2_v <= 1'b0; p3_v <= 1'b0; p4_v <= 1'b0;
                    p5_v <= 1'b0; p6_v <= 1'b0;
                    st <= T_LOOP;
                end else if (step == 2'd3 ||
                             (step == 2'd2 && !(is_r || do_ms))) begin
                    sfb <= sfb + 6'd1;
                    st <= T_BAND;
                end else
                    step <= step + 2'd1;
            end
            T_VMUL: begin
                m_a <= {{10{1'b0}}, gm};
                m_b <= isq;
                st <= T_VMUL2;
            end
            T_VMUL2: begin
                // m_p is written at the end of this cycle.
                st <= T_NOISE;
                pend <= 4'd3;
            end
            T_NOISE: begin
                if (pend == 4'd3) begin
                    pend <= 4'd2;           // m_q is being written
                end else if (pend == 4'd2) begin
                    pend <= 4'd1;
                    vsh_r <= vshift;
                end else if (pend != 4'd0) begin
                    pend <= 4'd0;
                    vval <= vnext;
                end else if (i == width) begin
                    step <= step + 2'd1;
                    st <= T_STEP;
                end else begin
                    prbs <= {prbs[29:0], prbs_bit};
                    if (step == 2'd0) begin
                        if (!joint && c) begin
                            r_we <= 1'b1; r_waddr <= base + {3'd0, i};
                            r_wdata <= prbs_bit ? 35'd0 - vval : vval;
                        end else begin
                            l_we <= 1'b1; l_waddr <= base + {3'd0, i};
                            l_wdata <= prbs_bit ? 35'd0 - vval : vval;
                        end
                    end else begin
                        r_we <= 1'b1; r_waddr <= base + {3'd0, i};
                        r_wdata <= prbs_bit ? 35'd0 - vval : vval;
                    end
                    i <= i + 7'd1;
                end
            end
            T_LOOP: begin
                // Issue a read a cycle; results come back in order.
                if (i != width) begin
                    l_raddr <= base + {3'd0, i};
                    r_raddr <= base + {3'd0, i};
                    i <= i + 7'd1;
                end
                p1_v <= i != width;
                p1_addr <= base + {3'd0, i};
                // p1: the address is in the RAM's register. p2: the data
                // is out of the RAM; M/S writes it back, intensity puts it
                // into the multiplier. p3: the product is being made. p4:
                // the product is out, and intensity writes it.
                p2_v <= p1_v; p2_addr <= p1_addr;
                p3_v <= p2_v; p3_addr <= p2_addr;
                p4_v <= p3_v; p4_addr <= p3_addr;
                p5_v <= p4_v; p5_addr <= p4_addr;
                p6_v <= p5_v; p6_addr <= p5_addr;
                // p5: the product is in m_q; shift it. p6: saturate, sign
                // and write.
                i_lo <= ishf[34:0];
                i_ovf <= !(ishf[69:34] == 36'd0 || ishf[69:34] == {36{1'b1}});
                i_sign <= ishf[69];
                i_pzero <= m_q == 60'sd0;
                i_psign <= m_q[59];
                if (p2_v) begin
                    if (is_r) begin
                        m_a <= l_rdata;
                        m_b <= gm;
                    end else begin
                        l_we <= 1'b1; l_waddr <= p2_addr; l_wdata <= sat36(msum);
                        r_we <= 1'b1; r_waddr <= p2_addr; r_wdata <= sat36(mdif);
                    end
                end
                if (is_r && p6_v) begin
                    r_we <= 1'b1; r_waddr <= p6_addr;
                    r_wdata <= ineg ? ineg_v : ival;
                end
                if (i == width && !p1_v && !p2_v && !p3_v && !p4_v && !p5_v &&
                    !p6_v) begin
                    step <= 2'd3;
                    st <= T_STEP;
                end
            end
            default: st <= T_IDLE;
            endcase
        end
    end
endmodule
