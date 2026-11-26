// h264_frame_addr_stub — where a reference sample lives in SDRAM, until
// `ip/memory/sdram_burst`'s `frame_addr` lands.
//
// The contract's tiled layout: one 32x32 tile is one SDRAM row of 512
// sixteen-bit words, two samples a word, the even sample in bits 7:0; luma
// and the two chroma planes are separate planes of the same tile shape. The
// word address of sample (x, y) inside its tile is 16 * (y % 32) + (x % 32)
// / 2, so a run of up to sixteen words along a tile row is one burst that
// stays in one SDRAM row, which is all `h264_inter` relies on.
//
// Where the tiles go is `frame_addr`'s business and this stub's choice is
// only a placeholder with the same ports: eight frames of 2^21 words each,
// luma tiles at a pitch of 64 a row, then Cb and Cr at a pitch of 32, and
// no bank swizzle. Swapping in the real module is changing the instance
// name; the test bench's memory model applies the same mapping.
module h264_frame_addr_stub #(
    parameter integer FB = 4                 // frame index bits
) (
    input  wire [FB-1:0] frame,
    input  wire [1:0]    plane,              // 0 Y, 1 Cb, 2 Cr
    input  wire [10:0]   x,                  // sample column
    input  wire [10:0]   y,                  // sample row
    output reg  [23:0]   addr                // 16-bit word address
);
    always @* begin
        if (plane == 2'd0)
            addr = {frame[2:0], 1'b0, y[9:5], x[10:5], y[4:0], x[4:1]};
        else
            addr = {frame[2:0], 1'b1, plane[1], 1'b0, y[8:5], x[9:5], y[4:0], x[4:1]};
    end
endmodule
