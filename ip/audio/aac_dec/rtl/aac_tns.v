// aac_tns -- temporal noise shaping (4.6.9), one channel at a time.
//
// The standard specifies the synthesis filter in direct form, after
// converting the transmitted reflection coefficients to LPC coefficients.
// This applies the same all-pole filter 1/A(z) as a lattice on the
// reflection coefficients themselves, which is the same transfer function
// with every coefficient bounded by one, so a Q1.23 coefficient is
// enough where the direct form's LPC coefficients can grow to hundreds
// (README, "TNS as a lattice"). For each sample x, order p:
//
//   f = x
//   for m = p .. 1:  f = f - k[m-1] * g[m-1]           (p products)
//   for m = p-1 .. 1: g[m] = k[m-1] * f_{m-1} + g[m-1]  (p - 1 products)
//   g[0] = f;  y = f
//
// One 35 x 25 multiplier, issued back to back within each half; about
// 2p + 6 cycles a sample. `FixedDecoder.tns` in the model is the
// arithmetic.

module aac_tns (
    input  wire        clk,
    input  wire        rst_n,
    input  wire        start,
    input  wire        ch,
    output reg         done,

    input  wire [3:0]  sfi,
    input  wire [1:0]  ws,
    input  wire [3:0]  ntns,

    input  wire        td_we,
    input  wire        td_ch,
    input  wire [2:0]  td_idx,
    input  wire [19:0] td_data,
    input  wire        tc_we,
    input  wire        tc_ch,
    input  wire [6:0]  tc_addr,
    input  wire [4:0]  tc_data,

    output reg  [9:0]  raddr,
    input  wire [34:0] rdata,
    output reg         we,
    output reg  [9:0]  waddr,
    output reg  [34:0] wdata
);
    // The filters and their coefficients, as the parser wrote them.
    reg [19:0] td_mem [0:15];
    reg [4:0]  tc_mem [0:255];
    reg [3:0]  td_raddr;
    reg [7:0]  tc_raddr;
    reg [19:0] td_q;
    reg [4:0]  tc_q;
    always @(posedge clk) begin
        if (td_we) td_mem[{td_ch, td_idx}] <= td_data;
        if (tc_we) tc_mem[{tc_ch, tc_addr}] <= tc_data;
        td_q <= td_mem[td_raddr];
        tc_q <= tc_mem[tc_raddr];
    end
    wire [24:0] refl;
    aac_tns_refl u_refl (.idx(tc_q), .q(refl));

    localparam [3:0]
        N_IDLE = 4'd0, N_DESC = 4'd1, N_DESC2 = 4'd2, N_DESC3 = 4'd3,
        N_COEF = 4'd4, N_SAMP = 4'd5, N_READ = 4'd6, N_READ2 = 4'd7,
        N_P = 4'd8, N_Q = 4'd9, N_OUT = 4'd10, N_NEXT = 4'd11;
    reg [3:0] st;
    reg       c;
    reg [2:0] di;          // descriptor
    reg [3:0] order;
    reg       dir;
    reg [2:0] w;
    reg [5:0] lo, hi;
    reg [9:0] addr;
    reg [10:0] left;       // samples left in this filter
    reg [4:0] ci;          // coefficient being loaded

    wire       short_w = ws == 2'd2;
    // The offsets of lo and hi, a cycle after they are set: N_COEF runs
    // at least two cycles before it reads them.
    wire [10:0] off_lo, off_hi;
    wire [5:0]  nsw, tmx;
    aac_swb u_swb (.clk(clk), .sfi(sfi), .short_win(short_w),
                   .band_a(lo), .band_b(hi),
                   .off_a(off_lo), .off_b(off_hi),
                   .num_swb(nsw), .tns_max(tmx));

    reg  signed [24:0] k  [0:11];
    reg  signed [34:0] gs [0:11];
    reg  [11:0] gval;      // which gs have been written since the filter began
    // gs has one write port: g[m] in the second half of a sample, g[0] at
    // its end.
    wire        gs_we = (st == N_Q && v3) || st == N_OUT;
    wire [3:0]  gs_wa = (st == N_OUT) ? 4'd0 : m3;
    wire [34:0] gs_wd = (st == N_OUT) ? f : g_new;
    always @(posedge clk)
        if (gs_we)
            gs[gs_wa] <= gs_wd;
    reg  signed [34:0] fv [0:11];
    reg  signed [34:0] f;

    // Multiplier, registered in and out, and its rounding registered: an
    // operand pair issued in cycle t is a product at the end of t + 1, a
    // rounded one at the end of t + 2, and used in t + 3.
    reg  signed [34:0] m_a;
    reg  signed [24:0] m_b;
    reg  signed [59:0] m_p;
    reg  signed [36:0] prod_r;
    wire signed [59:0] prod_full = (m_p + 60'sd4194304) >>> 23;
    always @(posedge clk) begin
        m_p <= m_a * m_b;
        prod_r <= prod_full[36:0];
    end
    reg  [3:0] iss;        // next index m to issue
    reg  [3:0] m1, m2, m3; // index of the product in flight, rounding, out
    reg        v1, v2, v3;
    function [34:0] sat37;
        input signed [36:0] v;
        begin
            if (v > 37'sh3_FFFF_FFFF)
                sat37 = 35'h3_FFFF_FFFF;
            else if (v < -37'sh4_0000_0000)
                sat37 = 35'h4_0000_0000;
            else
                sat37 = v[34:0];
        end
    endfunction
    wire signed [36:0] f_sub = {{2{f[34]}}, f} - prod_r;
    wire [34:0] f_new = sat37(f_sub);
    wire signed [34:0] g_prev = gval[m3 - 4'd1] ? gs[m3 - 4'd1] : 35'sd0;
    wire signed [36:0] g_add = prod_r + {{2{g_prev[34]}}, g_prev};
    wire [34:0] g_new = sat37(g_add);

    // The coefficient and state registers need no reset: every filter
    // loads k before it runs, reads a g it has not written as zero (gval),
    // and writes fv in the first half of each sample before the second half
    // reads it. So each is a small RAM, not flip-flops with a clear.

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            st <= N_IDLE; done <= 1'b0; c <= 1'b0; di <= 3'd0; order <= 4'd0;
            dir <= 1'b0; w <= 3'd0; lo <= 6'd0; hi <= 6'd0; addr <= 10'd0;
            left <= 11'd0; ci <= 5'd0; f <= 35'sd0;
            td_raddr <= 4'd0; tc_raddr <= 8'd0;
            m_a <= 35'sd0; m_b <= 25'sd0; iss <= 4'd0;
            m1 <= 4'd0; m2 <= 4'd0; m3 <= 4'd0; v1 <= 1'b0; v2 <= 1'b0; v3 <= 1'b0;
            gval <= 12'd0;
            raddr <= 10'd0; we <= 1'b0; waddr <= 10'd0; wdata <= 35'd0;
        end else begin
            done <= 1'b0;
            we <= 1'b0;
            case (st)
            N_IDLE: begin
                if (start) begin
                    c <= ch;
                    di <= 3'd0;
                    st <= (ntns == 4'd0) ? N_NEXT : N_DESC;
                    if (ntns == 4'd0) begin
                        done <= 1'b1;
                        st <= N_IDLE;
                    end
                end
            end
            N_DESC: begin
                td_raddr <= {c, di};
                st <= N_DESC2;
            end
            N_DESC2: st <= N_DESC3;
            N_DESC3: begin
                w     <= td_q[19:17];
                lo    <= td_q[16:11];
                hi    <= td_q[10:5];
                dir   <= td_q[4];
                order <= td_q[3:0];
                ci    <= 5'd0;
                tc_raddr <= {c, di, 4'd0};
                st    <= N_COEF;
            end
            N_COEF: begin
                // In cycle ci the RAM holds coefficient ci - 1 (its address
                // went in a cycle before), so k[ci - 1] loads here.
                tc_raddr <= {c, di, ci[3:0] + 4'd1};
                if (ci >= 5'd1)
                    k[ci - 5'd1] <= refl;
                if (ci == {1'b0, order}) begin
                    // Range: [start, end) in the window.
                    begin : range
                        reg [10:0] s0, e0;
                        s0 = short_w ? {1'b0, w, 7'd0} + off_lo : off_lo;
                        e0 = short_w ? {1'b0, w, 7'd0} + off_hi : off_hi;
                        addr <= dir ? e0[9:0] - 10'd1 : s0[9:0];
                        left <= e0 - s0;
                    end
                    gval <= 12'd0;     // every g starts at zero
                    st <= N_READ;
                end
                ci <= ci + 5'd1;
            end
            N_READ: begin
                raddr <= addr;
                st <= N_READ2;
                ci <= 5'd0;
            end
            N_READ2: begin
                // the address is in the RAM; its word is out next cycle
                ci <= ci + 5'd1;
                if (ci == 5'd1) begin
                    f <= rdata;
                    iss <= order;
                    v1 <= 1'b0; v2 <= 1'b0; v3 <= 1'b0;
                    st <= N_P;
                end
            end
            N_P: begin
                // Issue k[m-1] * g[m-1] for m = order .. 1.
                v1 <= iss != 4'd0;
                m1 <= iss;
                v2 <= v1;
                m2 <= m1;
                v3 <= v2;
                m3 <= m2;
                if (iss != 4'd0) begin
                    m_a <= gval[iss - 4'd1] ? gs[iss - 4'd1] : 35'sd0;
                    m_b <= k[iss - 4'd1];
                    iss <= iss - 4'd1;
                end
                if (v3) begin
                    f <= f_new;
                    fv[m3 - 4'd1] <= f_new;
                end
                if (iss == 4'd0 && !v1 && !v2 && !v3) begin
                    iss <= order - 4'd1;
                    st <= N_Q;
                end
            end
            N_Q: begin
                // Issue k[m-1] * f_{m-1} for m = order - 1 .. 1.
                v1 <= iss != 4'd0;
                m1 <= iss;
                v2 <= v1;
                m2 <= m1;
                v3 <= v2;
                m3 <= m2;
                if (iss != 4'd0) begin
                    m_a <= fv[iss - 4'd1];
                    m_b <= k[iss - 4'd1];
                    iss <= iss - 4'd1;
                end
                if (v3)
                    gval[m3] <= 1'b1;
                if (iss == 4'd0 && !v1 && !v2 && !v3)
                    st <= N_OUT;
            end
            N_OUT: begin
                gval[0] <= 1'b1;
                we <= 1'b1;
                waddr <= addr;
                wdata <= f;
                addr <= dir ? addr - 10'd1 : addr + 10'd1;
                left <= left - 11'd1;
                if (left == 11'd1)
                    st <= N_NEXT;
                else
                    st <= N_READ;
            end
            N_NEXT: begin
                if ({1'b0, di} + 4'd1 >= ntns) begin
                    done <= 1'b1;
                    st <= N_IDLE;
                end else begin
                    di <= di + 3'd1;
                    st <= N_DESC;
                end
            end
            default: st <= N_IDLE;
            endcase
        end
    end
endmodule
