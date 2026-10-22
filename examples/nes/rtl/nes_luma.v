// nes_luma — a 2C02 palette index as an eight-bit luma sample.
//
// What it does
//   `mjpeg_enc` is a greyscale encoder and the console's picture is a
//   six-bit palette index, so something has to turn one into the other.
//   This does, in one combinational step: the index goes to
//   `ppu_palette`, which is the same instance of the same table the DVI
//   and VGA tops use, and its RGB is weighted into a luma.
//
//   **There is no second palette here on purpose.** A 64-entry luma
//   table would have been smaller than this arithmetic, and it would
//   have been the palette written twice — the thing `mjpeg_enc`'s own
//   header warns about for quantisation tables. A table that can
//   disagree with itself eventually does.
//
//   The weights are BT.601's, at eight bits:
//
//     Y = (77 R + 150 G + 29 B) / 256
//
//   0.299, 0.587 and 0.114 times 256 are 76.5, 150.3 and 29.2, and
//   77 + 150 + 29 is exactly 256, so white is 255 and black is 0 and
//   nothing can overflow. Each product is a sum of shifts, because
//   three multipliers for a constant is three multipliers wasted:
//
//     77  = 64 + 8 + 4 + 1
//     150 = 128 + 16 + 4 + 2
//     29  = 16 + 8 + 4 + 1
//
//   The division truncates. Rounding would cost an adder and move
//   nothing by more than a level of 255.
//
// What it does not do
//   No gamma, and no full-range/studio-range distinction: the samples
//   JPEG wants are full range, 0 to 255, which is what this gives.
module nes_luma (
    input  wire [5:0] index,
    output wire [7:0] y
);
    wire [7:0] r;
    wire [7:0] g;
    wire [7:0] b;

    ppu_palette u_palette (
        .index (index),
        .r     (r),
        .g     (g),
        .b     (b)
    );

    // Twelve shifted copies, added at sixteen bits. The widest term is
    // G << 7, which is fifteen bits, and the total cannot exceed
    // 255 * 256 = 65280.
    wire [15:0] weighted =
          {2'b0, r, 6'b0} + {5'b0, r, 3'b0} + {6'b0, r, 2'b0} + {8'b0, r}
        + {1'b0, g, 7'b0} + {4'b0, g, 4'b0} + {6'b0, g, 2'b0} + {7'b0, g, 1'b0}
        + {4'b0, b, 4'b0} + {5'b0, b, 3'b0} + {6'b0, b, 2'b0} + {8'b0, b};

    assign y = weighted[15:8];
endmodule
