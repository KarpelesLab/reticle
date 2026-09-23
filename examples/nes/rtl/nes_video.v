// nes_video — the console's picture on a 640 x 480 screen.
//
// What it does
//   The console draws 256 x 240 pixels at its own rate, 341 dots by 262
//   lines of which 256 by 240 are visible, and a monitor wants 640 x 480
//   at 60 Hz. The two rasters have nothing to do with each other, so
//   this block puts a frame buffer between them: the console writes one
//   pixel per dot into it, and the transmitter reads one pixel per pixel
//   clock out of it.
//
//   Doubled, 256 x 240 is 512 x 480, which fills the height of a 640 x
//   480 frame exactly and leaves 64 pixels of border either side. So the
//   mapping is: the 512 columns from 64 to 575 are the console's 256,
//   two screen pixels each; all 480 rows are its 240, two each. Outside
//   that band the output is black.
//
//   Both sides run on `clk`, so there is no clock crossing here at all:
//   the console side moves on `en`, which `nes_top` gives it once every
//   24 cycles, and the transmitter side moves every cycle. What that
//   costs is tearing — the console writes a frame every 17.0 ms and the
//   screen reads one every 16.7 ms, so the two edges walk past each
//   other about twice a minute — and what it buys is one clock domain
//   and no synchronisers.
//
//   The frame buffer holds the console's six-bit palette index, not a
//   colour: 64 Ki entries of six bits, addressed by the pixel's Y and X
//   put together. `ppu_palette` turns the index into RGB one pixel
//   later, which is inside the four cycles of `clk` the transmitter
//   gives a fetch.
//
// What it does not do
//   No scaling other than two, no aspect correction — a real console's
//   pixels are not square, and these are — and no filtering. One frame
//   buffer, not two, so nothing waits for a frame boundary.
module nes_video (
    input  wire        clk,

    // The console's side: one pixel per `en` while `vid_de` is high.
    input  wire        en,
    input  wire        vid_de,
    input  wire [7:0]  vid_x,
    input  wire [7:0]  vid_y,
    input  wire [5:0]  vid_color,

    // The transmitter's side: the pixel it is fetching, and the colour
    // it is given for it.
    input  wire [11:0] x,
    input  wire [11:0] y,
    input  wire        de,
    output wire [7:0]  r,
    output wire [7:0]  g,
    output wire [7:0]  b
);
    // The doubled picture, centred: columns 64 to 575 of 640.
    localparam [11:0] LEFT  = 12'd64;
    localparam [11:0] RIGHT = 12'd576;

    wire [11:0] inside  = x - LEFT;
    wire        in_band = de & (x >= LEFT) & (x < RIGHT);
    wire [7:0]  nes_x   = inside[8:1];
    wire [7:0]  nes_y   = y[8:1];

    // One write port and one read port, which is the shape a block RAM
    // has. The two never collide in a way anyone can see: a torn frame
    // is a pixel of the new picture in the old one, not an unknown.
    reg [5:0] fb [0:65535];
    reg [5:0] fb_q;
    reg       band_q;

    always @(posedge clk) begin
        if (en & vid_de) fb[{vid_y, vid_x}] <= vid_color;
    end

    // No reset on the read side: a block RAM has nowhere to put one, and
    // a memory whose read sits in a process with an asynchronous reset
    // is read asynchronously as far as the mapper is concerned, which
    // would cost this one its 24 KiB of block RAM. The first pixel or
    // two after configuration are whatever the RAM came up with, which
    // is a frame the console has not drawn yet anyway.
    always @(posedge clk) begin
        fb_q   <= fb[{nes_y, nes_x}];
        band_q <= in_band;
    end

    wire [7:0] pr;
    wire [7:0] pg;
    wire [7:0] pb;

    ppu_palette u_palette (
        .index (fb_q),
        .r     (pr),
        .g     (pg),
        .b     (pb)
    );

    assign r = band_q ? pr : 8'd0;
    assign g = band_q ? pg : 8'd0;
    assign b = band_q ? pb : 8'd0;
endmodule
