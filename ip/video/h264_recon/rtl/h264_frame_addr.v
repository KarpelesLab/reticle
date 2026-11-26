// h264_frame_addr — (frame, plane, x, y) to an SDRAM word address, in
// the tiled layout of the decoder's memory contract.
//
// **A stand-in.** The contract gives this job to `frame_addr` in
// `ip/memory/sdram_burst`, which chooses the bank swizzle every client
// shares; until that module exists, this one keeps `h264_frame_wr`
// buildable and testable, and replacing it is replacing this file.
//
// What it does, combinationally
//   A tile is 32x32 samples of one plane, two samples a 16-bit word with
//   the even sample in bits 7:0: 16 words a tile row, 512 words a tile,
//   which is one 1024-byte SDRAM row. Within a tile, word
//   (y % 32) * 16 + (x % 32) / 2. Tiles are in raster order with a
//   stride of 64 tiles a row for luma (2048 samples) and 32 for chroma
//   (1024), and at most 16 tile rows of chroma (512 rows). A frame is
//   2^21 words: luma from 0, Cb from 2^20, Cr from 2^20 + 2^19. Eight
//   frames fill the 32 MB part, so `frame` is taken modulo 8. There is
//   no bank swizzle.
module h264_frame_addr (
    input  wire [3:0]  frame,
    input  wire [1:0]  plane,
    input  wire [10:0] x,
    input  wire [10:0] y,
    output wire [23:0] addr
);
    wire [20:0] luma   = {y[10:5], x[10:5], y[4:0], x[4:1]};
    wire [20:0] chroma = {1'b1, plane[1], 1'b0, y[8:5], x[9:5], y[4:0], x[4:1]};
    assign addr = {frame[2:0], (plane == 2'd0) ? luma : chroma};
endmodule
