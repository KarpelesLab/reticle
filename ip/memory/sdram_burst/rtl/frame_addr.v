// frame_addr — where a pixel of a tiled frame lives in the SDRAM.
//
// What it does
//   Maps (frame, plane, x, y) to the sixteen-bit word address that
//   `sdram_burst` takes, for the tiled layout every client of the
//   decoder shares. `ip/memory/sdram_burst/README.md` §"The frame
//   layout" is the specification; this module is its only
//   implementation, so a client never does the arithmetic itself.
//
//   A tile is 32 x 32 pixels of one plane and fills exactly one SDRAM
//   row: 512 words, two pixels a word, the even x in bits 7:0. Inside
//   the tile the word is `y % 32` times 16 plus `(x % 32) / 2`, so a
//   tile's line is sixteen consecutive words and a 2D block inside one
//   tile is all in one open row.
//
//   The word address is {row[12:0], bank[1:0], column[8:0]}, the split
//   `sdram_burst` uses. Tile (tx, ty) of a plane is in bank
//   (tx + 2*ty) mod 4, so the four tiles of any 2 x 2 square are in four
//   different banks: left, right, up, down and the diagonals are never
//   in the tile's own bank, and a block straddling a corner can have all
//   four of its rows open at once.
//
//   Rows. Within its bank a tile is row
//       BASE_ROW + frame * FRAME_ROWS + plane base + ty * (tiles across / 4) + tx / 4
//   where "tiles across / 4" is rounded up, and the plane bases are 0
//   for luma, LUMA_ROWS for Cb and LUMA_ROWS + CHROMA_ROWS for Cr.
//
//   `run` is how many words remain from this pixel's word to the right
//   edge of its tile's line, 1 to 16: the longest burst that may start
//   here without leaving the row or wrapping to the next line.
//
// What it does not do
//   No check that x and y are inside the picture, or that the frame
//   fits in the part: (8192 - BASE_ROW) / FRAME_ROWS frames do, 23 at
//   1280 x 720. It is combinational; a client registers around it.
module frame_addr #(
    // Luma tiles across and down: 1280 x 720 is 40 x 23 (720 / 32 is
    // 22.5, and the half tile is stored whole).
    parameter TW       = 40,
    parameter TH       = 23,
    // The first row of each bank the frames use.
    parameter BASE_ROW = 0,
    // Derived: never override these.
    parameter TWQ          = (TW + 3) / 4,
    parameter CTW          = (TW + 1) / 2,
    parameter CTH          = (TH + 1) / 2,
    parameter CTWQ         = (CTW + 3) / 4,
    parameter LUMA_ROWS    = TH * TWQ,
    parameter CHROMA_ROWS  = CTH * CTWQ,
    parameter FRAME_ROWS   = LUMA_ROWS + 2 * CHROMA_ROWS
) (
    input  wire [4:0]  frame,
    // 0 luma, 1 Cb, 2 Cr.
    input  wire [1:0]  plane,
    // The pixel, in the plane's own coordinates.
    input  wire [10:0] x,
    input  wire [10:0] y,
    output wire [23:0] addr,
    output wire [4:0]  run
);
    wire [5:0]  tx = x[10:5];
    wire [5:0]  ty = y[10:5];
    wire        chroma = (plane != 2'd0);

    localparam [12:0] P_CB = LUMA_ROWS;
    localparam [12:0] P_CR = LUMA_ROWS + CHROMA_ROWS;
    localparam [12:0] FR   = FRAME_ROWS;
    localparam [12:0] Q_L  = TWQ;
    localparam [12:0] Q_C  = CTWQ;
    localparam [12:0] BR   = BASE_ROW;

    wire [12:0] base  = (plane == 2'd0) ? 13'd0 : (plane == 2'd1) ? P_CB : P_CR;
    wire [12:0] f13   = {8'd0, frame};
    wire [12:0] ty13  = {7'd0, ty};
    // Two products by a constant, not one by a choice of two: the second
    // would be a multiplier.
    wire [12:0] l_luma = ty13 * Q_L;
    wire [12:0] l_chr  = ty13 * Q_C;
    wire [12:0] lines  = chroma ? l_chr : l_luma;
    wire [12:0] row   = BR + f13 * FR + base + lines + {9'd0, tx[5:2]};
    wire [1:0]  bank  = tx[1:0] + {ty[0], 1'b0};

    assign addr = {row, bank, y[4:0], x[4:1]};
    assign run  = 5'd16 - {1'b0, x[4:1]};
endmodule
