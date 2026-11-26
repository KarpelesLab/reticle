// aac_synth -- the filterbank (4.6.11): IMDCT, window, overlap-add, PCM.
//
// One channel per `start`. The IMDCT of M coefficients (M = 1024, or
// eight times 128) is a DCT-IV, computed with an M/2-point complex FFT:
//
//   u[n] = (X[2n] + i X[M-1-2n]) * W[n] / 2,  W[n] = exp(-i pi (n + 1/8) / M)
//   U    = FFT(u), each radix-2 stage halving
//   y[k] = U[k] * W[k];  D[2k] = Re y[k],  D[M-1-2k] = -Im y[k]
//
// and D is the IMDCT's 2M outputs with the standard's 2/N already in
// (README, "The IMDCT"): x[n] = D[n + M/2], -D[3M/2 - 1 - n] or
// -D[n - 3M/2], by range. The window then multiplies, the first half adds
// the previous frame's tail, and the second half becomes the next tail.
//
// One complex multiplier (four 35 x 25 products) serves every step: a
// pre-twiddle a cycle, a butterfly every two (it has two words to write),
// a post-twiddle a cycle and a windowed sample a cycle. Every step goes
// through the same six-stage pipeline:
//
//   0 issue: RAM and ROM addresses       3 the sums, registered
//   1 operands from the RAMs, registered 4 rounded and saturated, registered
//   2 the four products, registered      5 the butterfly's sums; written
//
// The work RAM holds 512 complex words: the FFT, then D in pairs
// {D[2k], D[M-1-2k]} at address k. `FixedDecoder.imdct` and `.synth` in
// the model are the arithmetic, bit for bit.

module aac_synth (
    input  wire        clk,
    input  wire        rst_n,
    input  wire        start,
    input  wire        ch,
    input  wire [1:0]  ws,
    input  wire        shape,
    input  wire        first,     // nothing before: no tail, no previous shape
    output reg         done,

    output reg  [9:0]  sp_raddr_a,
    output reg  [9:0]  sp_raddr_b,
    input  wire [34:0] sp_rdata_a,
    input  wire [34:0] sp_rdata_b,

    output reg         pcm_we,
    output reg         pcm_ch,
    output reg  [9:0]  pcm_n,
    output reg  [15:0] pcm_data
);
    // ------------------------------------------------------------------
    // Memories
    // ------------------------------------------------------------------
    reg         wk_we;
    reg  [8:0]  wk_waddr;
    reg  [69:0] wk_wdata;
    reg  [8:0]  wk_raddr_a, wk_raddr_b;
    wire [69:0] wk_a, wk_b;
    aac_ram #(.WIDTH(70), .DEPTH(512), .AW(9)) u_work (
        .clk(clk), .we(wk_we), .waddr(wk_waddr), .wdata(wk_wdata),
        .raddr_a(wk_raddr_a), .rdata_a(wk_a),
        .raddr_b(wk_raddr_b), .rdata_b(wk_b)
    );

    reg  [9:0]  tw_addr;
    wire [49:0] tw_q;
    aac_tw_rom u_tw (.clk(clk), .addr(tw_addr), .q(tw_q));

    reg  [11:0] win_addr;
    wire [24:0] win_q;
    aac_win_rom u_win (.clk(clk), .addr(win_addr), .q(win_q));

    reg         tl_we;
    reg  [10:0] tl_waddr;
    reg  [34:0] tl_wdata;
    reg  [10:0] tl_raddr;
    wire [34:0] tl_q, tl_unused;
    aac_ram #(.WIDTH(35), .DEPTH(2048), .AW(11)) u_tail (
        .clk(clk), .we(tl_we), .waddr(tl_waddr), .wdata(tl_wdata),
        .raddr_a(tl_raddr), .rdata_a(tl_q),
        .raddr_b(11'd0), .rdata_b(tl_unused)
    );

    reg         ov_we;
    reg  [6:0]  ov_waddr;
    reg  [34:0] ov_wdata;
    reg  [6:0]  ov_raddr;
    wire [34:0] ov_q, ov_unused;
    aac_ram #(.WIDTH(35), .DEPTH(128), .AW(7)) u_ovl (
        .clk(clk), .we(ov_we), .waddr(ov_waddr), .wdata(ov_wdata),
        .raddr_a(ov_raddr), .rdata_a(ov_q),
        .raddr_b(7'd0), .rdata_b(ov_unused)
    );

    // ------------------------------------------------------------------
    // Sequencing
    // ------------------------------------------------------------------
    localparam [2:0] K_PRE = 3'd0, K_FFT = 3'd1, K_POST = 3'd2,
                     K_WIN = 3'd3, K_ZERO = 3'd4, K_FLUSH = 3'd5;

    localparam [3:0]
        P_IDLE = 4'd0, P_PRE = 4'd1, P_FFT = 4'd2, P_POST = 4'd3,
        P_WIN = 4'd4, P_DRAIN = 4'd5, P_ZERO = 4'd6, P_FLUSH = 4'd7,
        P_ZTAIL = 4'd8, P_DONE = 4'd9, P_BLOCK = 4'd10;
    reg [3:0]  ph;
    reg [3:0]  ph_next;    // after a drain
    reg        c;
    reg        lng;        // a long window_sequence
    reg [1:0]  seq;
    reg        shp;
    reg        prev;       // shape of the left half
    reg        fst;
    reg        prev0, prev1;
    reg [2:0]  j;          // short window
    reg [10:0] n;          // the step's counter
    reg [3:0]  s;          // FFT stage
    reg [4:0]  drain;
    reg [10:0] zp;         // position of a ZERO or FLUSH item

    wire [9:0] qn  = lng ? 10'd512 : 10'd64;      // FFT points
    wire [3:0] qlg = lng ? 4'd9 : 4'd6;
    wire [9:0] base = lng ? 10'd0 : {j, 7'd0};
    wire [9:0] twb  = lng ? 10'd0 : 10'd512;       // pre/post twiddles

    // bit reversal of n, nine or six bits
    wire [8:0] n9 = n[8:0];
    wire [8:0] brev9 = {n9[0], n9[1], n9[2], n9[3], n9[4], n9[5], n9[6], n9[7], n9[8]};
    wire [8:0] brev6 = {3'd0, n9[0], n9[1], n9[2], n9[3], n9[4], n9[5]};

    // FFT butterfly b = n: t0 = b with a zero inserted at bit s, t1 = t0 + half
    wire [8:0] half  = 9'd1 << s;
    wire [8:0] lmask = half - 9'd1;
    wire [8:0] t0    = n[8:0] + (n[8:0] & ~lmask);
    wire [8:0] t1    = t0 + half;
    wire [7:0] twj   = (n[7:0] & lmask[7:0]) << (4'd8 - s);

    // Window position: which D word, negated or not, and which coefficient.
    // Long: n = 0..2047, M = 1024. Short: n = 0..255, M = 128.
    reg  [8:0]  d_addr;
    reg         d_im;      // the odd D's are in the high half... see below
    reg         d_neg;
    reg  [1:0]  w_const;   // 0 ROM, 1 one, 2 zero
    reg  [11:0] w_rom;
    always @(*) begin : winpos
        reg [11:0] p;     // n + M/2
        reg [10:0] m;     // D index
        reg [10:0] mm;
        reg [10:0] n2;
        p = lng ? ({1'b0, n} + 12'd512) : ({4'd0, n[7:0]} + 12'd64);
        if (lng) begin
            if (p < 12'd1024)      begin m = p[10:0];            d_neg = 1'b0; end
            else if (p < 12'd2048) begin m = 11'd2047 - p[10:0]; d_neg = 1'b1; end
            else                   begin m = p[10:0];            d_neg = 1'b1; end   // p - 2048
        end else begin
            if (p < 12'd128)       begin m = p[10:0];                     d_neg = 1'b0; end
            else if (p < 12'd256)  begin m = 11'd255 - p[10:0];           d_neg = 1'b1; end
            else                   begin m = p[10:0] - 11'd256;           d_neg = 1'b1; end
        end
        // D[m]: even m is the real part of word m/2, odd m the imaginary
        // part of word (M - 1 - m) / 2.
        mm = (lng ? 11'd1023 : 11'd127) - m;
        d_im   = m[0];
        d_addr = m[0] ? mm[9:1] : m[9:1];
        // The coefficient.
        n2 = lng ? (11'd2047 - n) : (11'd255 - {3'd0, n[7:0]});
        w_const = 2'd0;
        w_rom = 12'd0;
        if (lng) begin
            if (n < 11'd1024) begin
                if (seq == 2'd3) begin              // LONG_STOP
                    if (n < 11'd448)       w_const = 2'd2;
                    else if (n < 11'd576)  w_rom = 12'd2048 + {4'd0, prev, 7'd0} + {1'b0, n - 11'd448};
                    else                   w_const = 2'd1;
                end else
                    w_rom = {1'b0, prev, n[9:0]};
            end else begin
                if (seq == 2'd1) begin              // LONG_START
                    if (n2 < 11'd448)      w_const = 2'd2;
                    else if (n2 < 11'd576) w_rom = 12'd2048 + {4'd0, shp, 7'd0} + {1'b0, n2 - 11'd448};
                    else                   w_const = 2'd1;
                end else
                    w_rom = {1'b0, shp, n2[9:0]};
            end
        end else begin
            if (n[7:0] < 8'd128)
                w_rom = 12'd2048 + {4'd0, (j == 3'd0) ? prev : shp, 7'd0} + {5'd0, n[6:0]};
            else
                w_rom = 12'd2048 + {4'd0, shp, 7'd0} + {5'd0, n2[6:0]};
        end
    end

    // ------------------------------------------------------------------
    // The pipeline
    // ------------------------------------------------------------------
    // stage 0: the issue's tags; the addresses are in the RAMs' registers
    reg        v0;
    reg  [2:0] k0;
    reg  [8:0] a0, b0;
    reg [10:0] p0;
    reg        im0, neg0;
    reg  [1:0] wc0;
    reg  [7:0] n0;
    // stage 1: the RAMs' words are out
    reg        v1;
    reg  [8:0] b1, b2, b3, b4;
    reg  [2:0] k1;
    reg  [8:0] a1;         // destination word, or D word
    reg [10:0] p1;         // output position
    reg        im1, neg1;
    reg  [1:0] wc1;
    reg  [7:0] n1;         // short-window sample index
    // stage 2
    reg        v2;
    reg  [2:0] k2;
    reg  [8:0] a2;
    reg [10:0] p2;
    reg  [7:0] n2r;
    reg signed [34:0] ar, ai;
    reg signed [24:0] wr, wi;
    reg signed [34:0] tre2, tim2;       // the FFT's top word
    // stage 3
    reg        v3;
    reg  [2:0] k3;
    reg  [8:0] a3;
    reg [10:0] p3;
    reg  [7:0] n3;
    reg signed [59:0] pr1, pr2, pr3, pr4;
    reg signed [34:0] tre3, tim3;
    // stage 4
    reg        v4;
    reg  [2:0] k4;
    reg  [8:0] a4;
    reg [10:0] p4;
    reg  [7:0] n4;
    reg signed [60:0] sre, sim;
    reg signed [34:0] tre4, tim4;
    // stage 5: rounded and saturated
    reg        v5;
    reg  [2:0] k5;
    reg  [8:0] a5, b5;
    reg [10:0] p5;
    reg  [7:0] n5;
    reg  [34:0] y5r, y5i, y5n;      // re, im, and -im saturated
    reg signed [34:0] tre5, tim5;
    // the FFT's second write, a cycle later
    reg        bw_v;
    reg  [8:0] bw_a;
    reg [69:0] bw_d;

    function [34:0] sat_n;
        input signed [61:0] v;
        begin
            if (v > 62'sh3_FFFF_FFFF)
                sat_n = 35'h3_FFFF_FFFF;
            else if (v < -62'sh4_0000_0000)
                sat_n = 35'h4_0000_0000;
            else
                sat_n = v[34:0];
        end
    endfunction

    // stage 4 rounding: >> 24 for the pre-twiddle, >> 23 otherwise
    wire [4:0] rsh = (k4 == K_PRE) ? 5'd24 : 5'd23;
    wire signed [61:0] rre = ($signed({sre[60], sre}) + (62'sd1 <<< (rsh - 5'd1))) >>> rsh;
    wire signed [61:0] rim = ($signed({sim[60], sim}) + (62'sd1 <<< (rsh - 5'd1))) >>> rsh;
    wire [34:0] yr = sat_n(rre);
    wire [34:0] yi = sat_n(rim);
    wire [34:0] yi_neg = (yi == 35'h4_0000_0000) ? 35'h3_FFFF_FFFF : (35'd0 - yi);
    // butterfly, from stage 5
    wire signed [35:0] bt_s = {tre5[34], tre5} + {y5r[34], y5r} + 36'sd1;
    wire signed [35:0] bt_d = {tre5[34], tre5} - {y5r[34], y5r} + 36'sd1;
    wire signed [35:0] bi_s = {tim5[34], tim5} + {y5i[34], y5i} + 36'sd1;
    wire signed [35:0] bi_d = {tim5[34], tim5} - {y5i[34], y5i} + 36'sd1;
    // (a + t + 1) >> 1 of two 35-bit values always fits 35 bits
    wire [34:0] top_r = bt_s[35:1];
    wire [34:0] bot_r = bt_d[35:1];
    wire [34:0] top_i = bi_s[35:1];
    wire [34:0] bot_i = bi_d[35:1];

    // the window stage's z, and the short windows' overlap
    wire signed [35:0] zsum = {y5r[34], y5r} + {ov_q[34], ov_q};
    wire [34:0] zshort = (j == 3'd0) ? y5r : sat_n({{26{zsum[35]}}, zsum});

    // overlap-add
    reg         o1_v, o2_v;
    reg  [10:0] o1_p, o2_p;
    reg  [34:0] o1_z, o2_z;
    reg         o2_tail;   // o2 reads the tail
    wire signed [35:0] osum = {o2_z[34], o2_z} + (fst ? 36'sd0 : {tl_q[34], tl_q});
    wire [34:0] o = sat_n({{26{osum[35]}}, osum});
    wire signed [35:0] orr = ($signed({o[34], o}) + 36'sd64) >>> 7;
    wire [15:0] pcm = (orr > 36'sd32767) ? 16'h7FFF :
                      (orr < -36'sd32768) ? 16'h8000 : orr[15:0];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            ph <= P_IDLE; ph_next <= P_IDLE; done <= 1'b0; c <= 1'b0; lng <= 1'b1;
            seq <= 2'd0; shp <= 1'b0; prev <= 1'b0; fst <= 1'b1; prev0 <= 1'b0; prev1 <= 1'b0;
            j <= 3'd0; n <= 11'd0; s <= 4'd0; drain <= 5'd0; zp <= 11'd0;
            sp_raddr_a <= 10'd0; sp_raddr_b <= 10'd0;
            wk_we <= 1'b0; wk_waddr <= 9'd0; wk_wdata <= 70'd0;
            wk_raddr_a <= 9'd0; wk_raddr_b <= 9'd0;
            tw_addr <= 10'd0; win_addr <= 12'd0;
            tl_we <= 1'b0; tl_waddr <= 11'd0; tl_wdata <= 35'd0; tl_raddr <= 11'd0;
            ov_we <= 1'b0; ov_waddr <= 7'd0; ov_wdata <= 35'd0; ov_raddr <= 7'd0;
            v0 <= 1'b0; k0 <= 3'd0; a0 <= 9'd0; b0 <= 9'd0; p0 <= 11'd0; im0 <= 1'b0;
            neg0 <= 1'b0; wc0 <= 2'd0; n0 <= 8'd0;
            v1 <= 1'b0; k1 <= 3'd0; a1 <= 9'd0; p1 <= 11'd0; im1 <= 1'b0; neg1 <= 1'b0;
            wc1 <= 2'd0; n1 <= 8'd0;
            b1 <= 9'd0; b2 <= 9'd0; b3 <= 9'd0; b4 <= 9'd0;
            v2 <= 1'b0; k2 <= 3'd0; a2 <= 9'd0; p2 <= 11'd0; n2r <= 8'd0;
            ar <= 35'sd0; ai <= 35'sd0; wr <= 25'sd0; wi <= 25'sd0;
            tre2 <= 35'sd0; tim2 <= 35'sd0;
            v3 <= 1'b0; k3 <= 3'd0; a3 <= 9'd0; p3 <= 11'd0; n3 <= 8'd0;
            pr1 <= 60'sd0; pr2 <= 60'sd0; pr3 <= 60'sd0; pr4 <= 60'sd0;
            tre3 <= 35'sd0; tim3 <= 35'sd0;
            v4 <= 1'b0; k4 <= 3'd0; a4 <= 9'd0; p4 <= 11'd0; n4 <= 8'd0;
            sre <= 61'sd0; sim <= 61'sd0; tre4 <= 35'sd0; tim4 <= 35'sd0;
            bw_v <= 1'b0; bw_a <= 9'd0; bw_d <= 70'd0;
            v5 <= 1'b0; k5 <= 3'd0; a5 <= 9'd0; b5 <= 9'd0; p5 <= 11'd0; n5 <= 8'd0;
            y5r <= 35'd0; y5i <= 35'd0; y5n <= 35'd0; tre5 <= 35'sd0; tim5 <= 35'sd0;
            o1_v <= 1'b0; o2_v <= 1'b0; o1_p <= 11'd0; o2_p <= 11'd0;
            o1_z <= 35'd0; o2_z <= 35'd0; o2_tail <= 1'b0;
            pcm_we <= 1'b0; pcm_ch <= 1'b0; pcm_n <= 10'd0; pcm_data <= 16'd0;
        end else begin
            done <= 1'b0;
            wk_we <= 1'b0;
            tl_we <= 1'b0;
            ov_we <= 1'b0;
            pcm_we <= 1'b0;
            v0 <= 1'b0;

            // ----------------------------------------------------------
            // Issue
            // ----------------------------------------------------------
            case (ph)
            P_IDLE: begin
                if (start) begin
                    c   <= ch;
                    lng <= ws != 2'd2;
                    seq <= ws;
                    shp <= shape;
                    fst <= first;
                    prev <= first ? shape : (ch ? prev1 : prev0);
                    j   <= 3'd0;
                    n   <= 11'd0;
                    zp  <= 11'd0;
                    ph  <= (ws == 2'd2) ? P_ZERO : P_PRE;
                end
            end
            P_ZERO: begin
                // short: positions 0..447 are the tail alone
                v0 <= 1'b1; k0 <= K_ZERO; p0 <= zp;
                zp <= zp + 11'd1;
                if (zp == 11'd447) begin
                    n <= 11'd0;
                    ph <= P_PRE;
                end
            end
            P_PRE: begin
                sp_raddr_a <= base + {n[8:0], 1'b0};
                sp_raddr_b <= base + (lng ? 10'd1023 : 10'd127) - {n[8:0], 1'b0};
                tw_addr <= twb + n[9:0];
                v0 <= 1'b1; k0 <= K_PRE;
                a0 <= lng ? brev9 : brev6;
                n <= n + 11'd1;
                if (n + 11'd1 == {1'b0, qn}) begin
                    n <= 11'd0;
                    s <= 4'd0;
                    drain <= 5'd10;
                    ph <= P_DRAIN;
                    ph_next <= P_FFT;
                end
            end
            P_FFT: begin
                // a butterfly every other cycle
                if (!n[10]) begin
                    wk_raddr_a <= t0;
                    wk_raddr_b <= t1;
                    tw_addr <= 10'd576 + {2'd0, twj};
                    v0 <= 1'b1; k0 <= K_FFT; a0 <= t0; b0 <= t1;
                    n[10] <= 1'b1;
                end else begin
                    n[10] <= 1'b0;
                    if ({1'b0, n[9:0]} + 11'd1 == {2'd0, qn[9:1]}) begin
                        n <= 11'd0;
                        drain <= 5'd10;
                        ph <= P_DRAIN;
                        if (s + 4'd1 == qlg)
                            ph_next <= P_POST;
                        else begin
                            ph_next <= P_FFT;
                        end
                        s <= s + 4'd1;
                    end else
                        n[9:0] <= n[9:0] + 10'd1;
                end
            end
            P_POST: begin
                wk_raddr_a <= n[8:0];
                tw_addr <= twb + n[9:0];
                v0 <= 1'b1; k0 <= K_POST; a0 <= n[8:0];
                n <= n + 11'd1;
                if (n + 11'd1 == {1'b0, qn}) begin
                    n <= 11'd0;
                    drain <= 5'd10;
                    ph <= P_DRAIN;
                    ph_next <= P_WIN;
                end
            end
            P_WIN: begin
                wk_raddr_a <= d_addr;
                win_addr <= w_rom;
                v0 <= 1'b1; k0 <= K_WIN;
                im0 <= d_im; neg0 <= d_neg; wc0 <= w_const;
                n0 <= n[7:0];
                p0 <= lng ? n : (11'd448 + {1'b0, j, 7'd0} + {3'd0, n[7:0]});
                n <= n + 11'd1;
                if (n + 11'd1 == (lng ? 11'd2048 : 11'd256)) begin
                    n <= 11'd0;
                    drain <= 5'd12;
                    ph <= P_DRAIN;
                    if (lng)
                        ph_next <= P_DONE;
                    else if (j == 3'd7) begin
                        ph_next <= P_FLUSH;
                        zp <= 11'd1472;
                    end else
                        ph_next <= P_BLOCK;
                end
            end
            P_BLOCK: begin
                j <= j + 3'd1;
                n <= 11'd0;
                ph <= P_PRE;
            end
            P_FLUSH: begin
                // the last short window's second half, then zeros
                v0 <= 1'b1; k0 <= K_FLUSH; p0 <= zp; n0 <= {1'b0, zp[6:0] - 7'd64};
                zp <= zp + 11'd1;
                if (zp == 11'd1599)
                    ph <= P_ZTAIL;
            end
            P_ZTAIL: begin
                v0 <= 1'b1; k0 <= K_ZERO; p0 <= zp;
                zp <= zp + 11'd1;
                if (zp == 11'd2047) begin
                    drain <= 5'd12;
                    ph <= P_DRAIN;
                    ph_next <= P_DONE;
                end
            end
            P_DRAIN: begin
                if (drain == 5'd0)
                    ph <= ph_next;
                else
                    drain <= drain - 5'd1;
            end
            P_DONE: begin
                if (c) prev1 <= shp; else prev0 <= shp;
                done <= 1'b1;
                ph <= P_IDLE;
            end
            default: ph <= P_IDLE;
            endcase

            // Stage 0 -> 1: the read
            v1 <= v0; k1 <= k0; a1 <= a0; b1 <= b0; p1 <= p0; im1 <= im0;
            neg1 <= neg0; wc1 <= wc0; n1 <= n0;

            // ----------------------------------------------------------
            // Stage 1 -> 2: operands
            // ----------------------------------------------------------
            v2 <= v1; k2 <= k1; a2 <= a1; b2 <= b1; p2 <= p1; n2r <= n1;
            case (k1)
            K_PRE: begin
                ar <= sp_rdata_a; ai <= sp_rdata_b;
                wr <= tw_q[49:25]; wi <= tw_q[24:0];
            end
            K_FFT: begin
                ar <= wk_b[69:35]; ai <= wk_b[34:0];
                wr <= tw_q[49:25]; wi <= tw_q[24:0];
                tre2 <= wk_a[69:35]; tim2 <= wk_a[34:0];
            end
            K_POST: begin
                ar <= wk_a[69:35]; ai <= wk_a[34:0];
                wr <= tw_q[49:25]; wi <= tw_q[24:0];
            end
            default: begin   // K_WIN, and the multiplier-less items
                begin : xsel
                    reg [34:0] x;
                    x = im1 ? wk_a[34:0] : wk_a[69:35];
                    ar <= neg1 ? ((x == 35'h4_0000_0000) ? 35'sh3_FFFF_FFFF : -$signed(x)) : $signed(x);
                end
                ai <= 35'sd0;
                wr <= (wc1 == 2'd1) ? 25'sh80_0000 : (wc1 == 2'd2) ? 25'sd0 : $signed(win_q);
                wi <= 25'sd0;
            end
            endcase

            // Stage 2 -> 3: products
            v3 <= v2; k3 <= k2; a3 <= a2; b3 <= b2; p3 <= p2; n3 <= n2r;
            pr1 <= ar * wr;
            pr2 <= ai * wi;
            pr3 <= ar * wi;
            pr4 <= ai * wr;
            tre3 <= tre2; tim3 <= tim2;

            // Stage 3 -> 4: sums. The overlap word of a short window's first
            // half (or of a FLUSH item) is read here, for stage 4.
            v4 <= v3; k4 <= k3; a4 <= a3; b4 <= b3; p4 <= p3; n4 <= n3;
            ov_raddr <= n3[6:0];
            sre <= {pr1[59], pr1} - {pr2[59], pr2};
            sim <= {pr3[59], pr3} + {pr4[59], pr4};
            tre4 <= tre3; tim4 <= tim3;

            // Stage 4 -> 5: the rounding and saturation registered
            v5 <= v4; k5 <= k4; a5 <= a4; b5 <= b4; p5 <= p4; n5 <= n4;
            y5r <= yr; y5i <= yi; y5n <= yi_neg;
            tre5 <= tre4; tim5 <= tim4;

            // ----------------------------------------------------------
            // Stage 5: write
            // ----------------------------------------------------------
            o1_v <= 1'b0;
            tl_raddr <= {c, p5[9:0]};
            if (bw_v) begin
                wk_we <= 1'b1; wk_waddr <= bw_a; wk_wdata <= bw_d;
            end
            bw_v <= 1'b0;
            if (v5) begin
                case (k5)
                K_PRE: begin
                    wk_we <= 1'b1; wk_waddr <= a5; wk_wdata <= {y5r, y5i};
                end
                K_FFT: begin
                    wk_we <= 1'b1; wk_waddr <= a5; wk_wdata <= {top_r, top_i};
                    bw_v <= 1'b1; bw_a <= b5; bw_d <= {bot_r, bot_i};
                end
                K_POST: begin
                    wk_we <= 1'b1; wk_waddr <= a5; wk_wdata <= {y5r, y5n};
                end
                K_WIN: begin
                    if (lng) begin
                        o1_v <= 1'b1; o1_p <= p5; o1_z <= y5r;
                    end else if (n5 < 8'd128) begin
                        o1_v <= 1'b1; o1_p <= p5; o1_z <= zshort;
                    end else begin
                        ov_we <= 1'b1; ov_waddr <= n5[6:0]; ov_wdata <= y5r;
                    end
                end
                K_FLUSH: begin
                    o1_v <= 1'b1; o1_p <= p5; o1_z <= ov_q;
                end
                default: begin  // K_ZERO
                    o1_v <= 1'b1; o1_p <= p5; o1_z <= 35'd0;
                end
                endcase
            end

            // ----------------------------------------------------------
            // Overlap-add: o1 reads the tail, o2 adds it and writes PCM
            // ----------------------------------------------------------
            o2_v <= o1_v; o2_p <= o1_p; o2_z <= o1_z;
            o2_tail <= o1_v && o1_p < 11'd1024;
            if (o1_v && o1_p >= 11'd1024) begin
                tl_we <= 1'b1;
                tl_waddr <= {c, o1_p[9:0]};
                tl_wdata <= o1_z;
            end
            if (o2_v && o2_tail) begin
                pcm_we <= 1'b1;
                pcm_ch <= c;
                pcm_n <= o2_p[9:0];
                pcm_data <= pcm;
            end
        end
    end
endmodule
