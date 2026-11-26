// aac_dequant -- inverse quantisation and scalefactor gain, one a cycle.
//
//   x = sign(q) * |q|^(4/3) * 2^((sf - 100) / 4)            (4.6.1, 4.6.2)
//
// |q|^(4/3) comes from a 1024-entry ROM of i^(4/3) * 2^20; above 1024 it is
// interpolated between two entries, since q = 8j + r gives
// q^(4/3) = 16 (j + r/8)^(4/3), and the factor 16 is four quarter-steps of
// gain. The gain is a Q1.23 mantissa 2^(f/4) and a shift. The result is the
// decoder's spectral format: 35 bits signed, 7 fractional, saturated.
//
// Seven stages; the pipeline never stalls, so a coefficient in is a word
// written to the spectral RAM seven cycles later. `tools/aac_dec_model.py`
// (`pow43_fixed`, `scale_gain`) is the arithmetic, bit for bit.

module aac_dequant (
    input  wire        clk,
    input  wire        rst_n,
    input  wire        c_valid,
    input  wire        c_ch,
    input  wire [9:0]  c_addr,
    input  wire [14:0] c_q,
    input  wire [7:0]  c_sf,
    input  wire        c_zero,
    output reg         w_valid,
    output reg         w_ch,
    output reg  [9:0]  w_addr,
    output reg  [34:0] w_data
);
    localparam [33:0] POW43_1024 = 34'h285145f32;

    // Stage 0: magnitude, clamp, ROM addresses, gain exponent.
    wire        neg_in = c_q[14];
    wire [14:0] mag_in = neg_in ? (15'd0 - c_q) : c_q;
    wire [12:0] a_in   = (mag_in > 15'd8191) ? 13'd8191 : mag_in[12:0];
    wire        big_in = a_in > 13'd1024;
    wire [9:0]  j_in   = a_in[12:3];

    reg         s0_v, s0_ch, s0_neg, s0_zero, s0_big, s0_is1024, s0_j1024;
    reg  [9:0]  s0_addr;
    reg  [2:0]  s0_r;
    reg  [9:0]  s0_g;      // sf - 100 (+16 above 1024), two's complement
    wire [33:0] rom_a, rom_b;
    aac_pow43_rom u_rom (
        .clk(clk),
        .addr_a(big_in ? j_in : a_in[9:0]),
        .addr_b(j_in + 10'd1),
        .q_a(rom_a), .q_b(rom_b)
    );

    // Stage 1: the interpolation.
    reg         s1_v, s1_ch, s1_neg, s1_zero;
    reg  [9:0]  s1_addr;
    reg  [9:0]  s1_g;
    reg  [33:0] s1_mant;
    wire [33:0] ta  = s0_is1024 ? POW43_1024 : rom_a;
    wire [33:0] tb  = s0_j1024 ? POW43_1024 : rom_b;
    wire [24:0] dlt = tb[24:0] - rom_a[24:0];   // < 2^24 for j >= 128
    wire [26:0] rd  = (s0_r[0] ? {2'd0, dlt} : 27'd0) +
                      (s0_r[1] ? {1'd0, dlt, 1'b0} : 27'd0) +
                      (s0_r[2] ? {dlt, 2'b0} : 27'd0);
    wire [33:0] interp = rom_a + {10'd0, rd[26:3]} + {33'd0, rd[2]};

    // Stage 2: the product with the gain mantissa.
    wire [24:0] gm;
    aac_gain_mant u_gm (.idx(s1_g[1:0]), .q(gm));
    reg         s2_v, s2_ch, s2_neg, s2_zero;
    reg  [9:0]  s2_addr;
    reg  [7:0]  s2_e;      // floor(g / 4), two's complement
    reg  signed [34:0] s2_a;
    reg  signed [24:0] s2_b;

    // Stage 3: the product registered.
    reg         s3_v, s3_ch, s3_neg, s3_zero;
    reg  [9:0]  s3_addr;
    reg  [7:0]  s3_e;
    reg  signed [59:0] s3_p;

    // Stage 3b: the product again, after the fabric adder that joins the
    // two DSP slices' halves.
    reg         s3b_v, s3b_ch, s3b_neg, s3b_zero;
    reg  [9:0]  s3b_addr;
    reg  [7:0]  s3b_e;
    reg  signed [59:0] s3b_p;

    // Stage 4: shift right by 36 - e, rounding half up, saturate, sign.
    // With six bits prepended the shift is 42 - e, which is 0..67.
    wire [7:0]  sh  = 8'd42 - s3b_e;
    wire [65:0] pe  = {s3b_p[58:0], 7'd0} >> 1;              // p << 6
    wire [65:0] hf  = (sh == 8'd0) ? 66'd0 : (66'd1 << (sh - 8'd1));
    wire [65:0] rs  = (pe + hf) >> sh;

    // Stage 5: saturate and sign, from the shifted magnitude registered.
    reg         s4_v, s4_ch, s4_neg, s4_zero, s4_ovf;
    reg  [9:0]  s4_addr;
    reg  [33:0] s4_rs;
    wire [33:0] mag = s4_ovf ? 34'h3_FFFF_FFFF : s4_rs;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            s0_v <= 1'b0; s0_ch <= 1'b0; s0_neg <= 1'b0; s0_zero <= 1'b0;
            s0_big <= 1'b0; s0_is1024 <= 1'b0; s0_j1024 <= 1'b0;
            s0_addr <= 10'd0; s0_r <= 3'd0; s0_g <= 10'd0;
            s1_v <= 1'b0; s1_ch <= 1'b0; s1_neg <= 1'b0; s1_zero <= 1'b0;
            s1_addr <= 10'd0; s1_g <= 10'd0; s1_mant <= 34'd0;
            s2_v <= 1'b0; s2_ch <= 1'b0; s2_neg <= 1'b0; s2_zero <= 1'b0;
            s2_addr <= 10'd0; s2_e <= 8'd0; s2_a <= 35'sd0; s2_b <= 25'sd0;
            s3_v <= 1'b0; s3_ch <= 1'b0; s3_neg <= 1'b0; s3_zero <= 1'b0;
            s3_addr <= 10'd0; s3_e <= 8'd0; s3_p <= 60'sd0;
            s3b_v <= 1'b0; s3b_ch <= 1'b0; s3b_neg <= 1'b0; s3b_zero <= 1'b0;
            s3b_addr <= 10'd0; s3b_e <= 8'd0; s3b_p <= 60'sd0;
            s4_v <= 1'b0; s4_ch <= 1'b0; s4_neg <= 1'b0; s4_zero <= 1'b0;
            s4_ovf <= 1'b0; s4_addr <= 10'd0; s4_rs <= 34'd0;
            w_valid <= 1'b0; w_ch <= 1'b0; w_addr <= 10'd0; w_data <= 35'd0;
        end else begin
            s0_v    <= c_valid;
            s0_ch   <= c_ch;
            s0_addr <= c_addr;
            s0_neg  <= neg_in;
            s0_zero <= c_zero || a_in == 13'd0;
            s0_big  <= big_in;
            s0_is1024 <= a_in == 13'd1024;
            s0_j1024  <= j_in == 10'd1023;
            s0_r    <= a_in[2:0];
            s0_g    <= {2'd0, c_sf} - 10'd100 + (big_in ? 10'd16 : 10'd0);

            s1_v <= s0_v; s1_ch <= s0_ch; s1_addr <= s0_addr;
            s1_neg <= s0_neg; s1_zero <= s0_zero; s1_g <= s0_g;
            s1_mant <= s0_big ? interp : ta;

            s2_v <= s1_v; s2_ch <= s1_ch; s2_addr <= s1_addr;
            s2_neg <= s1_neg; s2_zero <= s1_zero;
            s2_e <= s1_g[9:2];   // floor(g / 4): -25..42 fits eight bits
            s2_a <= {1'b0, s1_mant};
            s2_b <= gm;

            s3_v <= s2_v; s3_ch <= s2_ch; s3_addr <= s2_addr;
            s3_neg <= s2_neg; s3_zero <= s2_zero; s3_e <= s2_e;
            s3_p <= s2_a * s2_b;

            s3b_v <= s3_v; s3b_ch <= s3_ch; s3b_addr <= s3_addr;
            s3b_neg <= s3_neg; s3b_zero <= s3_zero; s3b_e <= s3_e;
            s3b_p <= s3_p;

            s4_v <= s3b_v; s4_ch <= s3b_ch; s4_addr <= s3b_addr;
            s4_neg <= s3b_neg; s4_zero <= s3b_zero;
            s4_ovf <= rs[65:34] != 32'd0;
            s4_rs <= rs[33:0];

            w_valid <= s4_v;
            w_ch    <= s4_ch;
            w_addr  <= s4_addr;
            w_data  <= s4_zero ? 35'd0 :
                       (s4_neg ? 35'd0 - {1'b0, mag} : {1'b0, mag});
        end
    end
endmodule
