// ppu_palette — one of the 2C02's 64 colours, as 24-bit RGB.
//
// What it does
//   Turns the six-bit palette index `ppu2c02` puts on `vid_color` into
//   a colour a monitor understands. Purely combinational: give it an
//   index, take the three bytes.
//
//   The 2C02 has no colours in it. It generates a composite signal
//   directly, from a hue (the low four bits of the index) and a
//   brightness (the next two), and what a screen showed depended on the
//   screen. So there is no single right answer and no table to copy:
//   what follows is **our own** conversion, laid out the way the part's
//   own index is laid out —
//
//     bits 3-0   hue.   $x0 is grey, $x1 through $xC walk the wheel
//                       from blue round to blue-green, $xD is a darker
//                       grey and $xE and $xF are black on every real
//                       part.
//     bits 5-4   level. $0x is dark, $1x and $2x brighter, $3x pale.
//
//   Read the table as four rows of sixteen and the wheel is visible in
//   it. The row of four blacks down the right-hand side is not a gap in
//   the table, it is the part: $xE and $xF really are black, and a
//   program that writes $0D into a palette entry gets a black darker
//   than the signal is supposed to go, which is why this table gives it
//   $000000 rather than something below it.
//
// What it does not do
//   No colour emphasis: $2001 bits 5-7 tint the composite signal on a
//   real part, and nothing here is analogue. No PAL or Dendy variant.
module ppu_palette (
    input  wire [5:0] index,
    output wire [7:0] r,
    output wire [7:0] g,
    output wire [7:0] b
);
    reg [23:0] rgb;

    always @* begin
        case (index)
            // Level $0x — the dark row.
            6'h00: rgb = 24'h54_54_54;
            6'h01: rgb = 24'h00_1E_74;
            6'h02: rgb = 24'h08_10_90;
            6'h03: rgb = 24'h30_00_88;
            6'h04: rgb = 24'h44_00_64;
            6'h05: rgb = 24'h5C_00_30;
            6'h06: rgb = 24'h54_04_00;
            6'h07: rgb = 24'h3C_18_00;
            6'h08: rgb = 24'h20_2A_00;
            6'h09: rgb = 24'h08_3A_00;
            6'h0A: rgb = 24'h00_40_00;
            6'h0B: rgb = 24'h00_3C_00;
            6'h0C: rgb = 24'h00_32_3C;
            6'h0D: rgb = 24'h00_00_00;
            6'h0E: rgb = 24'h00_00_00;
            6'h0F: rgb = 24'h00_00_00;
            // Level $1x.
            6'h10: rgb = 24'h98_96_98;
            6'h11: rgb = 24'h08_4C_C4;
            6'h12: rgb = 24'h30_32_EC;
            6'h13: rgb = 24'h5C_1E_E4;
            6'h14: rgb = 24'h88_14_B0;
            6'h15: rgb = 24'hA0_14_64;
            6'h16: rgb = 24'h98_22_20;
            6'h17: rgb = 24'h78_3C_00;
            6'h18: rgb = 24'h54_5A_00;
            6'h19: rgb = 24'h28_72_00;
            6'h1A: rgb = 24'h08_7C_00;
            6'h1B: rgb = 24'h00_76_28;
            6'h1C: rgb = 24'h00_66_78;
            6'h1D: rgb = 24'h00_00_00;
            6'h1E: rgb = 24'h00_00_00;
            6'h1F: rgb = 24'h00_00_00;
            // Level $2x — the bright row most graphics live in.
            6'h20: rgb = 24'hEC_EE_EC;
            6'h21: rgb = 24'h4C_9A_EC;
            6'h22: rgb = 24'h78_7C_EC;
            6'h23: rgb = 24'hB0_62_EC;
            6'h24: rgb = 24'hE4_54_EC;
            6'h25: rgb = 24'hEC_58_B4;
            6'h26: rgb = 24'hEC_6A_64;
            6'h27: rgb = 24'hD4_88_20;
            6'h28: rgb = 24'hA0_AA_00;
            6'h29: rgb = 24'h74_C4_00;
            6'h2A: rgb = 24'h4C_D0_20;
            6'h2B: rgb = 24'h38_CC_6C;
            6'h2C: rgb = 24'h38_B4_CC;
            6'h2D: rgb = 24'h3C_3C_3C;
            6'h2E: rgb = 24'h00_00_00;
            6'h2F: rgb = 24'h00_00_00;
            // Level $3x — the pale row.
            6'h30: rgb = 24'hEC_EE_EC;
            6'h31: rgb = 24'hA8_CC_EC;
            6'h32: rgb = 24'hBC_BC_EC;
            6'h33: rgb = 24'hD4_B2_EC;
            6'h34: rgb = 24'hEC_AE_EC;
            6'h35: rgb = 24'hEC_AE_D4;
            6'h36: rgb = 24'hEC_B4_B0;
            6'h37: rgb = 24'hE4_C4_90;
            6'h38: rgb = 24'hCC_D2_78;
            6'h39: rgb = 24'hB4_DE_78;
            6'h3A: rgb = 24'hA8_E2_90;
            6'h3B: rgb = 24'h98_E2_B4;
            6'h3C: rgb = 24'hA0_D6_E4;
            6'h3D: rgb = 24'hA0_A2_A0;
            6'h3E: rgb = 24'h00_00_00;
            default: rgb = 24'h00_00_00;
        endcase
    end

    assign r = rgb[23:16];
    assign g = rgb[15:8];
    assign b = rgb[7:0];
endmodule
