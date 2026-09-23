// apple2_video — the Apple II video scanner, on a 640x480 raster.
//
// What it does
//   Turns the pixel a raster is asking for into a colour, by reading the
//   machine's RAM the way an Apple II's video counters read it. It is
//   driven by `dvi_tx`'s side of the conversation: `x`, `y` and `de` name
//   the pixel being fetched and advance on `pix_en`, and the colour for
//   that pixel is expected on `r`, `g`, `b` before the next one. Every
//   register here moves on `pix_en`, so the block behaves the same at one
//   pixel per clock (the testbench) and at one pixel in five (the board),
//   which is what `dvi_tx` gives it.
//
// The picture
//   The Apple II draws 40 columns of 24 rows in cells seven dots wide and
//   eight scan lines tall: 280 x 192. Doubled in both directions that is
//   560 x 384, which fits inside 640 x 480 with a 40-pixel border left and
//   right and a 48-line border above and below. So a character cell is 14
//   pixels wide and 16 lines tall on this raster, and everything outside
//   [40,600) x [48,432) is black.
//
// The text page, and why its lines are not in order
//   The text page is 1 KiB at $0400 and the base address of line N is
//   *not* $0400 + 40*N. The Apple II's video counter is a shift register
//   that produces the address of line N as
//
//       $0400 + 128 * (N mod 8) + 40 * (N div 8)
//
//   so the screen is three interleaved groups of eight lines: $400, $480,
//   $500 ... $780 are lines 0, 1, 2 ... 7, then $428, $4A8 ... are lines
//   8, 9 ... and $450, $4D0 ... are lines 16, 17 ... The 8 x 8 = 64 bytes
//   the arithmetic never reaches are the famous "screen holes", which on a
//   real machine belong to the peripheral slots.
//
//   The form of that formula is what makes it cheap in hardware: 128 *
//   (N mod 8) is the low three bits of the row moved up to bits 9..7, and
//   40 * (N div 8) + column is a seven-bit number, because the column is
//   less than 40 and the multiplier is 0, 40 or 80. So the address is a
//   concatenation and one small adder, which is `vaddr` below — no
//   multiply, no table.
//
// The character generator
//   A byte of the text page is a *display code*, not ASCII. Its low six
//   bits pick one of 64 glyphs — glyph 00..1F are ASCII $40..$5F and
//   glyph 20..3F are ASCII $20..$3F, which is the order sw/font.txt is
//   written in — and its top two bits are the attribute:
//
//       byte[7] = 1            normal      (display codes $80..$FF)
//       byte[7:6] = 2'b00      inverse     (display codes $00..$3F)
//       byte[7:6] = 2'b01      flashing    (display codes $40..$7F)
//
//   Inverse swaps dot and background; flashing does it every few frames.
//   `flash_count` counts frames and its bit FLASH_SHIFT is the phase, so
//   a flashing cell spends 2**FLASH_SHIFT frames one way and as many the
//   other — by default eight and eight, about 3.7 Hz at 60 Hz, near
//   enough the rate a real machine flashes at.
//
// What it does not do
//   **No graphics at all.** Lo-res and hi-res are not here, so $C050 to
//   $C057 are not decoded (see rtl/apple2.v) and the screen is always the
//   text page at $0400. That is a scope line and not an oversight:
//   `tests/apple2.rs` proves what is on the screen by decoding a whole
//   640 x 480 frame out of the video signal, one frame costs a good part
//   of a minute of simulation, and a mode nothing decodes back off the
//   wire would be a mode nobody had checked. README.md says so plainly.
//
//   No 80-column card, no alternate character set, no lowercase: the font
//   is ASCII $20..$5F, which is what an original Apple II could show. No
//   PAGE2, for the same reason as the graphics.
//
// The pipeline
//   Two clocked lookups stand between a byte of RAM and a dot on the
//   screen — the text page and then the character generator — and a dot
//   is one pixel. So the fetch runs a whole character cell ahead of the
//   dots being drawn, and the counters start fourteen pixels before the
//   left edge of the picture:
//
//       hc == 0    `vaddr` names the cell being fetched (`fcol`)
//       hc == 1    RAM has answered; latch the byte and address the font
//       hc == 3    the font has answered; latch the row of dots
//       hc == 13   hand the row of dots to the shifter and step `fcol`
//
//   `hc` counts 0..13 across the fourteen pixels of one cell, so `hc[3:1]`
//   is which of the seven dots is being drawn and `hc[0]` is which half of
//   the doubled pixel. The cell the *dots* come from is one behind
//   `fcol`, which is why the lead-in cell at x = 26..39 is fetched with
//   nothing displayed: it is the one that fills the shifter for column 0.
module apple2_video #(
    // The character generator, as sw/font.txt says it: 512 bytes, eight
    // rows for each of 64 glyphs, the leftmost dot in bit 0.
    parameter FONT_FILE   = "sw/font.hex",
    // Which bit of the frame counter drives the flashing attribute, so a
    // flashing cell spends 2**FLASH_SHIFT frames in each state. 3 is
    // sixteen frames a cycle, about 3.7 Hz at 60 Hz, which is near enough
    // the rate a real machine flashes at. A testbench turns it down so it
    // can see both states without simulating sixteen frames of video.
    parameter FLASH_SHIFT = 3
) (
    input  wire        clk,
    input  wire        rst_n,
    // The raster, from dvi_tx.
    input  wire        pix_en,
    input  wire        de,
    input  wire [11:0] x,
    input  wire [11:0] y,

    // The video side of the machine's RAM: an address, a request for one
    // byte a cell, and the byte back one `pix_en` later. One fetch per
    // character cell is what a real Apple II does — its video reads one
    // byte per processor cycle — and it keeps the read port quiet for
    // thirteen pixels in fourteen.
    output wire [15:0] vaddr,
    output wire        vfetch,
    input  wire [7:0]  vdata,

    // The colour of the pixel `x`, `y` names.
    output reg  [7:0]  r,
    output reg  [7:0]  g,
    output reg  [7:0]  b
);
    // The picture's place on the raster. A cell is CELL_W pixels wide
    // because each of its seven dots is drawn twice.
    localparam [11:0] CELL_W  = 14;
    localparam [11:0] X_LEFT  = 40;             // (640 - 40*14) / 2
    localparam [11:0] X_RIGHT = X_LEFT + 560;
    localparam [11:0] Y_TOP   = 48;             // (480 - 24*16) / 2
    localparam [11:0] Y_BOT   = Y_TOP + 384;
    // Where the fetch counters start: one whole cell before the picture,
    // so column 0's dots are in the shifter when x reaches X_LEFT. The
    // comparison is against the pixel *before* that, because the counters
    // move on the same edge the raster does.
    localparam [11:0] X_GEN   = X_LEFT - CELL_W - 1;
    // The last pixel and the last line of the raster, which is where the
    // frame counter that drives the flashing attribute ticks over.
    localparam [11:0] X_LAST  = 799;
    localparam [11:0] Y_LAST  = 524;
    // A lit dot. The Apple II's monitor was monochrome in text; so is
    // this, and a cell is either this colour or black.
    localparam [23:0] WHITE   = 24'hFFFFFF;

    // -----------------------------------------------------------------
    // Where we are on the screen
    // -----------------------------------------------------------------
    reg [3:0] hc;     // 0..13, the pixel inside a character cell
    reg [5:0] fcol;   // 0..39, the column being fetched
    reg [4:0] row;    // 0..23, the text row
    reg [2:0] sub;    // 0..7, the scan line inside the row
    reg       vhalf;  // which of the two scan lines a doubled line is

    wire in_h  = (x >= X_LEFT) & (x < X_RIGHT);
    wire in_v  = (y >= Y_TOP) & (y < Y_BOT);
    wire eol   = (x == X_LAST);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            hc   <= 4'd0;
            fcol <= 6'd0;
        end else if (pix_en) begin
            if (x == X_GEN) begin
                hc   <= 4'd0;
                fcol <= 6'd0;
            end else if (hc == 4'd13) begin
                hc   <= 4'd0;
                fcol <= (fcol == 6'd39) ? 6'd0 : fcol + 6'd1;
            end else begin
                hc <= hc + 4'd1;
            end
        end
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            row   <= 5'd0;
            sub   <= 3'd0;
            vhalf <= 1'b0;
        end else if (pix_en & eol) begin
            if (y == Y_TOP - 1) begin
                row   <= 5'd0;
                sub   <= 3'd0;
                vhalf <= 1'b0;
            end else if (y < Y_BOT - 1) begin
                vhalf <= ~vhalf;
                if (vhalf) begin
                    sub <= sub + 3'd1;
                    if (sub == 3'd7) row <= row + 5'd1;
                end
            end
        end
    end

    // -----------------------------------------------------------------
    // The address: $0400 (or $0800) + 128 * (row mod 8) + 40 * (row div 8)
    //              + column, as a concatenation and one seven-bit adder.
    // -----------------------------------------------------------------
    reg [6:0] group;
    always @* begin
        case (row[4:3])
            2'd0:    group = 7'd0;
            2'd1:    group = 7'd40;
            default: group = 7'd80;
        endcase
    end
    wire [6:0] seg = group + {1'b0, fcol};
    assign vaddr = {4'b0000, 2'b01, row[2:0], seg};
    // The one pixel of the cell in which the RAM is read: the byte lands
    // in `vdata` at the next edge, which is hc == 1.
    assign vfetch = (hc == 4'd0);

    // -----------------------------------------------------------------
    // The character generator
    // -----------------------------------------------------------------
    reg [7:0] font [0:511];
    initial $readmemh(FONT_FILE, font);

    reg [8:0] faddr;
    reg [7:0] fq;
    always @(posedge clk) begin
        // Once a cell, in the pixel after `faddr` is set, so `fq` is there
        // to be latched at hc == 3.
        if (pix_en & (hc == 4'd2)) fq <= font[faddr];
    end

    // -----------------------------------------------------------------
    // The flashing attribute: eight frames lit, eight frames not.
    // -----------------------------------------------------------------
    reg [3:0] flash_count;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) flash_count <= 4'd0;
        else if (pix_en & eol & (y == Y_LAST)) flash_count <= flash_count + 4'd1;
    end
    wire flash = flash_count[FLASH_SHIFT];

    // -----------------------------------------------------------------
    // The pipeline: fetch a cell ahead, then shift its dots out.
    // -----------------------------------------------------------------
    reg [7:0] chr;    // the display code of the cell being fetched
    reg [6:0] nbits;  // its seven dots, leftmost in bit 0
    reg       ninv;   // whether they come out inverted
    reg [6:0] bits;   // the same two, for the cell being drawn
    reg       inv;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            chr   <= 8'd0;
            faddr <= 9'd0;
            nbits <= 7'd0;
            ninv  <= 1'b0;
            bits  <= 7'd0;
            inv   <= 1'b0;
        end else if (pix_en) begin
            // RAM answered the address `vaddr` has been holding all cell.
            if (hc == 4'd1) begin
                chr   <= vdata;
                faddr <= {vdata[5:0], sub};
            end
            // The character generator answered.
            if (hc == 4'd3) begin
                nbits <= fq[6:0];
                // Inverse always, flashing on the lit half of the frame
                // counter, normal never.
                ninv  <= ~chr[7] & (~chr[6] | flash);
            end
            // The cell being drawn ends; the one just fetched starts.
            if (hc == 4'd13) begin
                bits <= nbits;
                inv  <= ninv;
            end
        end
    end

    // -----------------------------------------------------------------
    // The dot
    // -----------------------------------------------------------------
    wire lit = bits[hc[3:1]] ^ inv;
    always @* begin
        if (de & in_h & in_v & lit) begin
            r = WHITE[23:16];
            g = WHITE[15:8];
            b = WHITE[7:0];
        end else begin
            r = 8'h00;
            g = 8'h00;
            b = 8'h00;
        end
    end
endmodule
