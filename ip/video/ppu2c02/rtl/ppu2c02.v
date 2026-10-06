// ppu2c02 — the picture processing unit of an NES-compatible console.
//
// What it does
//   The publicly documented behaviour of the Ricoh 2C02: a 256 x 240
//   raster drawn out of a tiled background and up to 64 sprites, the
//   eight registers a processor reaches it through, and the fetch
//   pipeline whose timing is what makes scrolling work.
//
//   This is a reimplementation from the published description of the
//   part — a machine's observable behaviour, not anyone's creative
//   work. No game data, no character data and no lockout logic is in
//   this package, and none is needed to run it.
//
//   The raster. 341 dots by 262 scanlines, one dot per `en`:
//
//     0-239    visible. Dots 1-256 each emit one pixel on `vid_*`.
//     240      post-render, nothing happens.
//     241      vblank begins at dot 1: the flag is set and `nmi` rises
//              if $2000 bit 7 says so.
//     242-260  vblank continues.
//     261      pre-render: the three status flags are cleared at dot 1,
//              vert(v) is reloaded during dots 280-304, and the first
//              two tiles of scanline 0 are prefetched at dots 321-336.
//
//   The registers, at $2000-$2007 of the processor's map. `reg_addr` is
//   the three address bits, `reg_we` and `reg_re` are one-`en` strobes,
//   and `reg_dout` is valid combinationally while `reg_addr` names a
//   readable one:
//
//     $2000 PPUCTRL   w   nametable, increment, pattern tables, NMI
//     $2001 PPUMASK   w   greyscale, the two left-column masks, the two
//                         enables
//     $2002 PPUSTATUS r   vblank, sprite 0 hit, sprite overflow; reading
//                         it clears vblank *and the write latch*
//     $2003 OAMADDR   w
//     $2004 OAMDATA   rw  a write stores and post-increments OAMADDR
//     $2005 PPUSCROLL w   two writes, fine X then the rest
//     $2006 PPUADDR   w   two writes, high six bits then low eight
//     $2007 PPUDATA   rw  through `v`, which then advances by 1 or 32
//
//   The internal registers are the ones the reverse-engineered
//   description names: `v` and `t`, fifteen bits each, `x`, three bits
//   of fine horizontal scroll, and `w`, **one** write latch shared by
//   $2005 and $2006. That sharing is the part everyone gets wrong: two
//   writes to $2005 and two to $2006 interleave through the same
//   toggle, and reading $2002 puts it back to zero, which is why every
//   NES program reads $2002 before it writes a scroll.
//
//     v/t layout   yyy NN YYYYY XXXXX
//                  ||| || ||||| +++++-- coarse X
//                  ||| || +++++-------- coarse Y
//                  ||| ++-------------- nametable select
//                  +++----------------- fine Y
//
//   The background fetch. Over each eight dots the part makes four
//   two-dot fetches — nametable byte, attribute byte, pattern low,
//   pattern high — and at the eighth dot it reloads the shift registers
//   and increments the coarse X of `v`. Dots 1-256 fetch two tiles
//   ahead of the pixel being drawn, and dots 321-336 of the *previous*
//   line prefetch the first two, which is what lines the pipeline up so
//   that pixel 0 of a line comes from the tile fetched at dots 321-328.
//   The pixel selects bit 15 - x of a sixteen-bit shift register, so
//   fine X costs nothing at all.
//
//   Sprites. 256 bytes of OAM inside the part, four bytes a sprite.
//   During dots 65-256 of a visible line the evaluator walks OAM one
//   byte per two dots looking for sprites whose Y puts them on the
//   *next* line, copies the first eight it finds, and sets the overflow
//   flag if a ninth is in range. During dots 257-320 the eight copied
//   sprites have their pattern bytes fetched, flipped if the attribute
//   says so. During the next line's dots 1-256 the eight compare their
//   X against the pixel, the lowest-numbered opaque one wins, and its
//   attribute bit 5 decides whether it is drawn over or under an opaque
//   background pixel. Sprite 0 hit is set on the dot where sprite
//   zero's own opaque pixel meets an opaque background pixel.
//
//   The bus. `vram_addr` is presented for the dot and `vram_din` is
//   expected on the *next* `en`, which is what a synchronous memory
//   does and what makes a two-dot fetch one address and one latch.
//   $0000-$1FFF is the cartridge's pattern memory and $2000-$3EFF the
//   console's nametable memory; the 32 bytes of palette at $3F00-$3FFF
//   are inside this block and never reach the bus.
//
//   The video port. `vid_de` is high for dots 1-256 of lines 0-239,
//   `vid_x` and `vid_y` name the pixel and `vid_color` is its six-bit
//   NES palette index. `ppu_palette` in this package turns that index
//   into RGB. With rendering disabled the port still runs and emits the
//   backdrop, which is what a blanked screen looks like.
//
// What it does not do
//   **8 x 16 sprites.** $2000 bit 5 is stored and ignored; every sprite
//   is eight by eight.
//
//   **The sprite overflow bug.** The flag is set when a ninth in-range
//   sprite is found, which is what the flag is *for*; the real part's
//   evaluator also increments the byte index when it should not, so it
//   both misses overflows and invents them. That bug is not reproduced.
//
//   **Colour emphasis.** $2001 bits 5-7 are stored and ignored, because
//   they tint an analogue signal this block does not generate.
//
//   **The odd-frame dot skip.** The real part drops dot 339 of the
//   pre-render line on odd frames when rendering is on, to keep the
//   colour burst from walking. Every frame here is 341 x 262 dots.
//
//   **Memory accesses through $2007 while rendering.** The real part
//   performs them, against an address its own fetch counter is
//   scribbling on, and the result is glitched pixels and a corrupted
//   `v`. Here a $2007 access during rendering leaves pattern and
//   nametable memory alone; `v` still advances, so a program that does
//   it sees the increment but not the access. Palette writes are not
//   on that bus and do happen.
//
//   **Open bus.** $2002's low five bits, $2007's top two in the palette
//   range and a read of a write-only register all give zero here, where
//   the real part gives whatever was last on its data bus.
//
//   **Sprites on scanline 0.** Evaluation does not run on the
//   pre-render line, so the topmost line a sprite can reach is 1. On
//   the real part a sprite is drawn one line below its OAM Y for the
//   same reason.
//
//   **OAMADDR's corners.** Evaluation always starts at sprite zero,
//   where the real part starts at OAMADDR and shuffles priorities when
//   a program leaves it elsewhere, and OAMADDR is not forced to zero
//   during the sprite fetches.
//
//   No analogue output, no NTSC or PAL colour encoding and no
//   composite artefacts: the output is a palette index per pixel.
module ppu2c02 (
    input  wire        clk,
    input  wire        rst_n,
    // One dot. The whole block advances on an `en` edge and on nothing
    // else, so a design running faster than the dot rate gives it a dot
    // enable rather than a second clock.
    input  wire        en,

    // The eight registers, already decoded out of $2000-$3FFF.
    input  wire [2:0]  reg_addr,
    input  wire [7:0]  reg_din,
    output reg  [7:0]  reg_dout,
    // One `en` wide, on the edge the processor's access completes.
    input  wire        reg_we,
    input  wire        reg_re,
    // Active high, so it is already the polarity `mos6502` wants.
    output wire        nmi,

    // Pattern and nametable memory, $0000-$3EFF. `vram_din` is the byte
    // at the address presented on the previous `en`.
    output reg  [13:0] vram_addr,
    output wire [7:0]  vram_dout,
    output wire        vram_we,
    input  wire [7:0]  vram_din,

    // The picture: one pixel per `en` while `vid_de` is high.
    output wire        vid_de,
    output wire [7:0]  vid_x,
    output wire [7:0]  vid_y,
    output wire [5:0]  vid_color,
    output wire        vid_frame,

    // Where the raster is, for a testbench.
    output wire [8:0]  dbg_dot,
    output wire [8:0]  dbg_line
);
    // -----------------------------------------------------------------
    // The raster
    // -----------------------------------------------------------------
    localparam [8:0] DOT_LAST  = 9'd340;
    localparam [8:0] LINE_LAST = 9'd261;

    reg [8:0] dot;
    reg [8:0] line;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            dot  <= 9'd0;
            line <= 9'd0;
        end else if (en) begin
            if (dot == DOT_LAST) begin
                dot  <= 9'd0;
                line <= (line == LINE_LAST) ? 9'd0 : line + 9'd1;
            end else begin
                dot <= dot + 9'd1;
            end
        end
    end

    assign dbg_dot  = dot;
    assign dbg_line = line;

    wire visible_line = (line < 9'd240);
    wire pre_line     = (line == LINE_LAST);
    wire visible_dot  = (dot >= 9'd1) && (dot <= 9'd256);

    // -----------------------------------------------------------------
    // The registers a processor writes
    // -----------------------------------------------------------------
    reg [7:0]  ctrl;   // $2000
    reg [7:0]  mask;   // $2001
    reg        st_vblank;
    reg        st_sprite0;
    reg        st_overflow;

    reg [14:0] v;
    reg [14:0] t;
    reg [2:0]  fine_x;
    reg        w;          // the one write latch $2005 and $2006 share
    reg [7:0]  read_buffer;
    reg        fill_buffer;
    reg [7:0]  oam_addr;

    // $2000 bit 2 picks how far a $2007 access moves `v`: down a row of
    // the nametable, or along it.
    wire [14:0] addr_step = ctrl[2] ? 15'd32 : 15'd1;

    // Rendering is on when either enable is set, and it only *does*
    // anything on a line that draws or prepares.
    wire rendering     = mask[3] | mask[4];
    wire render_line   = visible_line | pre_line;
    wire rendering_now = rendering & render_line;

    // $3F00-$3FFF is the palette, which lives here and not on the bus.
    wire pal_sel = (v[13:8] == 6'b111111);

    assign nmi = st_vblank & ctrl[7];

    // -----------------------------------------------------------------
    // Fetch windows
    //
    // Background: dots 1-256 draw and fetch, dots 321-336 prefetch the
    // next line's first two tiles. Sprites: dots 257-320, eight dots a
    // sprite.
    // -----------------------------------------------------------------
    wire       bg_window = ((dot >= 9'd1)   && (dot <= 9'd256))
                         || ((dot >= 9'd321) && (dot <= 9'd336));
    wire       sp_window = (dot >= 9'd257) && (dot <= 9'd320);

    wire [8:0] bg_off    = dot - 9'd1;
    wire [2:0] bg_phase  = bg_off[2:0];
    wire [8:0] sp_off    = dot - 9'd257;
    wire [2:0] sp_slot   = sp_off[5:3];
    wire [2:0] sp_phase  = sp_off[2:0];

    // The eighth dot of a background group: reload and step coarse X.
    wire bg_step = rendering_now & bg_window & (bg_phase == 3'd7);

    // -----------------------------------------------------------------
    // The background pipeline
    // -----------------------------------------------------------------
    reg [7:0]  nt_byte;
    reg [7:0]  at_byte;
    reg [7:0]  pat_lo;

    reg [15:0] sr_lo;
    reg [15:0] sr_hi;
    reg [15:0] sr_al;   // attribute bit 0, one bit per pixel
    reg [15:0] sr_ah;   // attribute bit 1

    // The attribute quadrant for the tile `v` names: bit 1 of coarse Y
    // picks the row of the pair, bit 1 of coarse X the column.
    reg [1:0] at_bits;
    always @* begin
        case ({v[6], v[1]})
            2'b00:   at_bits = at_byte[1:0];
            2'b01:   at_bits = at_byte[3:2];
            2'b10:   at_bits = at_byte[5:4];
            default: at_bits = at_byte[7:6];
        endcase
    end

    // -----------------------------------------------------------------
    // The sprites of the line being drawn, and the ones found for the
    // next one. The two sets are the part's two OAMs: the fetch during
    // dots 257-320 is what moves a sprite from the second to the first.
    // -----------------------------------------------------------------
    reg [7:0] sp_cnt   [0:7];   // dots still to walk before the sprite
    reg [3:0] sp_rem   [0:7];   // pixels of it still to come
    reg [7:0] sp_lo    [0:7];
    reg [7:0] sp_hi    [0:7];
    reg [1:0] sp_pal   [0:7];
    reg       sp_prio  [0:7];
    reg       sp_zero  [0:7];
    reg [7:0] sp_plane0;

    reg [7:0] se_tile  [0:7];
    reg [7:0] se_x     [0:7];
    reg [7:0] se_attr  [0:7];
    reg [2:0] se_row   [0:7];
    reg       se_zero  [0:7];
    reg [3:0] se_count;

    reg [5:0] eval_n;      // which sprite the evaluator is reading
    reg [1:0] eval_m;      // which of its four bytes
    reg       eval_done;

    wire eval_window = rendering_now & visible_line
                     & (dot >= 9'd65) & (dot <= 9'd256);
    // One byte per two dots, as the part does it: the odd dot presents
    // the address, the even dot takes the byte.
    wire eval_step   = eval_window & ~eval_done & (dot[0] == 1'b0);

    // -----------------------------------------------------------------
    // Object attribute memory: one write port and one registered read
    // port, so it is the shape a block RAM has.
    //
    // The read address is the evaluator's while it is working and
    // OAMADDR otherwise, which is why a read of $2004 during rendering
    // returns whatever byte the evaluation is on — the same thing the
    // real part does, for the same reason.
    // -----------------------------------------------------------------
    reg  [7:0] oam [0:255];
    reg  [7:0] oam_q;
    wire [7:0] oam_raddr = eval_window ? {eval_n, eval_m} : oam_addr;
    wire [7:0] eval_diff = line[7:0] - oam_q;

    // The read port is on its own, with no reset on it: a block RAM has
    // nowhere to put one, and a memory whose read sits in a process with
    // an asynchronous reset is read asynchronously as far as the mapper
    // is concerned, which costs it the block RAM it should have had.
    always @(posedge clk) begin
        if (en) oam_q <= oam[oam_raddr];
    end

    // -----------------------------------------------------------------
    // Palette memory: 32 six-bit entries. $3F10, $3F14, $3F18 and $3F1C
    // are not entries of their own, they are the backdrop seen through
    // the sprite half, so bits 1-0 being zero forces bit 4 low.
    // -----------------------------------------------------------------
    reg [5:0] pal [0:31];

    function [4:0] pal_index;
        input [4:0] a;
        begin
            pal_index = (a[1:0] == 2'b00) ? {1'b0, a[3:0]} : a;
        end
    endfunction

    wire [5:0] pal_read = pal[pal_index(v[4:0])];

    // -----------------------------------------------------------------
    // The address on the bus for this dot
    // -----------------------------------------------------------------
    wire [13:0] nt_addr = {2'b10, v[11:0]};
    wire [13:0] at_addr = {2'b10, v[11:10], 4'b1111, v[9:7], v[4:2]};
    wire [13:0] bg_pat  = {1'b0, ctrl[4], nt_byte, 1'b0, v[14:12]};

    // The slot the sprite fetch is on. Verilog has no bit-select of an
    // array element to spare us the copies, so the word comes out once.
    wire [7:0]  sp_attr_now = se_attr[sp_slot];
    wire [7:0]  sp_tile_now = se_tile[sp_slot];
    wire [2:0]  sp_row_now  = se_row[sp_slot];
    wire [7:0]  sp_x_now    = se_x[sp_slot];
    wire        sp_zero_now = se_zero[sp_slot];

    // Horizontal flip is the byte read backwards, which is where the
    // part does it too: the shift register only ever shifts one way.
    wire [7:0]  sp_fetch_lo = sp_attr_now[6]
                            ? {sp_plane0[0], sp_plane0[1], sp_plane0[2],
                               sp_plane0[3], sp_plane0[4], sp_plane0[5],
                               sp_plane0[6], sp_plane0[7]}
                            : sp_plane0;
    wire [7:0]  sp_fetch_hi = sp_attr_now[6]
                            ? {vram_din[0], vram_din[1], vram_din[2],
                               vram_din[3], vram_din[4], vram_din[5],
                               vram_din[6], vram_din[7]}
                            : vram_din;

    // Vertical flip picks the row from the other end of the tile.
    wire [2:0]  sp_line = sp_attr_now[7] ? (3'd7 - sp_row_now) : sp_row_now;
    wire [13:0] sp_pat  = {1'b0, ctrl[3], sp_tile_now, 1'b0, sp_line};

    always @* begin
        if (rendering_now && bg_window) begin
            case (bg_phase)
                3'd0, 3'd1: vram_addr = nt_addr;
                3'd2, 3'd3: vram_addr = at_addr;
                3'd4, 3'd5: vram_addr = bg_pat;
                default:    vram_addr = bg_pat | 14'h0008;
            endcase
        end else if (rendering_now && sp_window) begin
            case (sp_phase)
                3'd4, 3'd5: vram_addr = sp_pat;
                3'd6, 3'd7: vram_addr = sp_pat | 14'h0008;
                default:    vram_addr = nt_addr;
            endcase
        end else begin
            // Nothing is fetching, so the bus is the processor's.
            vram_addr = v[13:0];
        end
    end

    assign vram_dout = reg_din;
    assign vram_we   = reg_we & (reg_addr == 3'd7) & ~pal_sel & ~rendering_now;

    // -----------------------------------------------------------------
    // The pixel
    // -----------------------------------------------------------------
    assign vid_de    = visible_line & visible_dot;
    assign vid_x     = dot[7:0] - 8'd1;
    assign vid_y     = line[7:0];
    assign vid_frame = (dot == 9'd0) & (line == 9'd0);

    wire [1:0] bg_bits = {sr_hi[15 - fine_x], sr_lo[15 - fine_x]};
    wire [1:0] bg_pal  = {sr_ah[15 - fine_x], sr_al[15 - fine_x]};
    wire       bg_show = mask[3] & (mask[1] | (vid_x >= 8'd8));
    wire       bg_op   = bg_show & (bg_bits != 2'b00);

    // A slot is putting out a pixel when its counter has reached the
    // sprite's X and it still has pixels left; the pixel is the top bit
    // of its pair of shift registers. The lowest-numbered opaque slot
    // wins, which is the priority the part gives them.
    reg [1:0] sp_bits;
    reg [1:0] sp_pl;
    reg       sp_behind;
    reg       sp_hit0;
    reg [7:0] sp_byte_lo;
    reg [7:0] sp_byte_hi;
    integer   s;
    always @* begin
        sp_bits    = 2'b00;
        sp_pl      = 2'b00;
        sp_behind  = 1'b0;
        sp_hit0    = 1'b0;
        sp_byte_lo = 8'd0;
        sp_byte_hi = 8'd0;
        for (s = 7; s >= 0; s = s - 1) begin
            sp_byte_lo = sp_lo[s];
            sp_byte_hi = sp_hi[s];
            if ((sp_cnt[s] == 8'd0) && (sp_rem[s] != 4'd0)
                && ({sp_byte_hi[7], sp_byte_lo[7]} != 2'b00)) begin
                sp_bits   = {sp_byte_hi[7], sp_byte_lo[7]};
                sp_pl     = sp_pal[s];
                sp_behind = sp_prio[s];
                sp_hit0   = sp_zero[s];
            end
        end
    end

    wire sp_show = mask[4] & (mask[2] | (vid_x >= 8'd8));
    wire sp_op   = sp_show & (sp_bits != 2'b00);

    reg [5:0] raw_color;
    always @* begin
        if (!rendering || (!bg_op && !sp_op)) begin
            raw_color = pal[5'd0];
        end else if (sp_op && (!bg_op || !sp_behind)) begin
            raw_color = pal[pal_index({1'b1, sp_pl, sp_bits})];
        end else begin
            raw_color = pal[pal_index({1'b0, bg_pal, bg_bits})];
        end
    end

    // Greyscale forces every colour onto the grey column of the palette.
    assign vid_color = mask[0] ? {raw_color[5:4], 4'b0000} : raw_color;

    // Sprite zero hit: sprite zero's own opaque pixel over an opaque
    // background one, anywhere but the last column.
    wire hit_now = rendering_now & visible_line & visible_dot
                 & bg_op & sp_op & sp_hit0 & (vid_x != 8'd255);

    // -----------------------------------------------------------------
    // The two halves of `v` the raster moves, written out because the
    // wrap-arounds are the whole of scrolling: coarse X past 31 flips
    // the nametable sideways, coarse Y past 29 flips it downwards, and
    // past 31 — which only a program that wrote a coarse Y of 30 or 31
    // can reach — wraps without flipping anything.
    // -----------------------------------------------------------------
    wire [14:0] inc_x = (v[4:0] == 5'd31)
                      ? {v[14:11], ~v[10], v[9:5], 5'd0}
                      : (v + 15'd1);
    reg  [14:0] inc_y;
    always @* begin
        if (v[14:12] != 3'd7) begin
            inc_y = v + 15'h1000;
        end else if (v[9:5] == 5'd29) begin
            inc_y = {3'd0, ~v[11], v[10], 5'd0, v[4:0]};
        end else if (v[9:5] == 5'd31) begin
            inc_y = {3'd0, v[11:10], 5'd0, v[4:0]};
        end else begin
            inc_y = {3'd0, v[11:10], v[9:5] + 5'd1, v[4:0]};
        end
    end

    integer i;
    integer k;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            ctrl        <= 8'd0;
            mask        <= 8'd0;
            st_vblank   <= 1'b0;
            st_sprite0  <= 1'b0;
            st_overflow <= 1'b0;
            v           <= 15'd0;
            t           <= 15'd0;
            fine_x      <= 3'd0;
            w           <= 1'b0;
            read_buffer <= 8'd0;
            fill_buffer <= 1'b0;
            oam_addr    <= 8'd0;
            nt_byte     <= 8'd0;
            at_byte     <= 8'd0;
            pat_lo      <= 8'd0;
            sp_plane0   <= 8'd0;
            sr_lo       <= 16'd0;
            sr_hi       <= 16'd0;
            sr_al       <= 16'd0;
            sr_ah       <= 16'd0;
            se_count    <= 4'd0;
            eval_n      <= 6'd0;
            eval_m      <= 2'd0;
            eval_done   <= 1'b0;
            // The palette is 32 entries of six bits and it is read four
            // ways at once, so it is a register file rather than a
            // memory and it can afford a reset. OAM is 256 bytes read
            // one at a time, so it is a block RAM and has none: a
            // program fills it before it shows a sprite.
            for (k = 0; k < 32; k = k + 1) pal[k] <= 6'd0;
            for (i = 0; i < 8; i = i + 1) begin
                sp_cnt[i]  <= 8'd0;
                sp_rem[i]  <= 4'd0;
                sp_lo[i]   <= 8'd0;
                sp_hi[i]   <= 8'd0;
                sp_pal[i]  <= 2'd0;
                sp_prio[i] <= 1'b0;
                sp_zero[i] <= 1'b0;
                se_tile[i] <= 8'd0;
                se_x[i]    <= 8'd0;
                se_attr[i] <= 8'd0;
                se_row[i]  <= 3'd0;
                se_zero[i] <= 1'b0;
            end
        end else if (en) begin
            // ---- the flags the raster owns -------------------------
            if ((line == 9'd241) && (dot == 9'd1)) st_vblank <= 1'b1;
            if (pre_line && (dot == 9'd1)) begin
                st_vblank   <= 1'b0;
                st_sprite0  <= 1'b0;
                st_overflow <= 1'b0;
            end
            if (hit_now) st_sprite0 <= 1'b1;

            // ---- the background pipeline ---------------------------
            if (rendering_now && bg_window) begin
                // Shift, and on the eighth dot take the pair of bytes
                // fetched over the last eight into the low half.
                if (bg_phase == 3'd7) begin
                    sr_lo <= {sr_lo[14:7], pat_lo};
                    sr_hi <= {sr_hi[14:7], vram_din};
                    sr_al <= {sr_al[14:7], {8{at_bits[0]}}};
                    sr_ah <= {sr_ah[14:7], {8{at_bits[1]}}};
                end else begin
                    sr_lo <= {sr_lo[14:0], 1'b0};
                    sr_hi <= {sr_hi[14:0], 1'b0};
                    sr_al <= {sr_al[14:0], 1'b0};
                    sr_ah <= {sr_ah[14:0], 1'b0};
                end
                case (bg_phase)
                    3'd1:    nt_byte <= vram_din;
                    3'd3:    at_byte <= vram_din;
                    3'd5:    pat_lo  <= vram_din;
                    default: ;
                endcase
            end

            // ---- what the raster does to `v` -----------------------
            // At dot 256 the vertical step wins over the horizontal
            // one, which cannot be seen: dot 257 reloads hori(v) from
            // `t` before anything reads it again.
            if (bg_step)                          v <= inc_x;
            if (rendering_now && (dot == 9'd256)) v <= inc_y;
            if (rendering_now && (dot == 9'd257)) begin
                v[10]  <= t[10];
                v[4:0] <= t[4:0];
            end
            if (rendering_now && pre_line
                && (dot >= 9'd280) && (dot <= 9'd304)) begin
                v[14:11] <= t[14:11];
                v[9:5]   <= t[9:5];
            end

            // ---- sprite evaluation ---------------------------------
            if (render_line && (dot == 9'd64)) begin
                se_count  <= 4'd0;
                eval_n    <= 6'd0;
                eval_m    <= 2'd0;
                eval_done <= 1'b0;
            end
            if (eval_step) begin
                if (eval_m == 2'd0) begin
                    // The byte is the sprite's Y, and a sprite is drawn
                    // on the eight lines *below* it.
                    if (eval_diff < 8'd8) begin
                        if (se_count < 4'd8) begin
                            se_row[se_count[2:0]]  <= eval_diff[2:0];
                            se_zero[se_count[2:0]] <= (eval_n == 6'd0);
                            eval_m                 <= 2'd1;
                        end else begin
                            st_overflow <= 1'b1;
                            if (eval_n == 6'd63) eval_done <= 1'b1;
                            eval_n <= eval_n + 6'd1;
                        end
                    end else begin
                        if (eval_n == 6'd63) eval_done <= 1'b1;
                        eval_n <= eval_n + 6'd1;
                    end
                end else if (eval_m == 2'd1) begin
                    se_tile[se_count[2:0]] <= oam_q;
                    eval_m                 <= 2'd2;
                end else if (eval_m == 2'd2) begin
                    se_attr[se_count[2:0]] <= oam_q;
                    eval_m                 <= 2'd3;
                end else begin
                    se_x[se_count[2:0]] <= oam_q;
                    se_count            <= se_count + 4'd1;
                    eval_m              <= 2'd0;
                    if (eval_n == 6'd63) eval_done <= 1'b1;
                    eval_n <= eval_n + 6'd1;
                end
            end

            // ---- the eight sprite slots ----------------------------
            // Each slot is the arrangement the part itself has: a
            // counter that walks down to the sprite's X and a pair of
            // shift registers that then put out one pixel a dot for
            // eight dots. No slot compares itself against the pixel
            // number — it counts — which is both what the silicon does
            // and eight comparators and eight barrel shifters cheaper.
            if (rendering_now && sp_window && (sp_phase == 3'd5)) begin
                sp_plane0 <= vram_din;
            end
            for (i = 0; i < 8; i = i + 1) begin
                if (rendering_now && sp_window && (sp_phase == 3'd7)
                    && (sp_slot == i[2:0])) begin
                    sp_lo[i]   <= sp_fetch_lo;
                    sp_hi[i]   <= sp_fetch_hi;
                    sp_cnt[i]  <= sp_x_now;
                    sp_pal[i]  <= sp_attr_now[1:0];
                    sp_prio[i] <= sp_attr_now[5];
                    sp_zero[i] <= sp_zero_now;
                    sp_rem[i]  <= ({1'b0, sp_slot} < se_count) ? 4'd8 : 4'd0;
                end else if (rendering_now && visible_line && visible_dot) begin
                    if (sp_cnt[i] != 8'd0) begin
                        sp_cnt[i] <= sp_cnt[i] - 8'd1;
                    end else if (sp_rem[i] != 4'd0) begin
                        sp_lo[i]  <= sp_lo[i] << 1;
                        sp_hi[i]  <= sp_hi[i] << 1;
                        sp_rem[i] <= sp_rem[i] - 4'd1;
                    end
                end
            end

            // ---- the deferred half of a $2007 read -----------------
            if (fill_buffer) begin
                read_buffer <= vram_din;
                fill_buffer <= 1'b0;
            end

            // ---- what the processor writes -------------------------
            // Last, so a write that lands on the same dot as one of the
            // raster's own reloads wins, which is the order the part
            // gives them.
            if (reg_we) begin
                case (reg_addr)
                    3'd0: begin
                        ctrl     <= reg_din;
                        t[11:10] <= reg_din[1:0];
                    end
                    3'd1: mask <= reg_din;
                    3'd3: oam_addr <= reg_din;
                    3'd4: begin
                        oam[oam_addr] <= reg_din;
                        oam_addr      <= oam_addr + 8'd1;
                    end
                    3'd5: begin
                        if (!w) begin
                            t[4:0] <= reg_din[7:3];
                            fine_x <= reg_din[2:0];
                        end else begin
                            t[14:12] <= reg_din[2:0];
                            t[9:5]   <= reg_din[7:3];
                        end
                        w <= ~w;
                    end
                    3'd6: begin
                        if (!w) begin
                            t[13:8] <= reg_din[5:0];
                            t[14]   <= 1'b0;
                        end else begin
                            t[7:0] <= reg_din;
                            v      <= {t[14:8], reg_din};
                        end
                        w <= ~w;
                    end
                    3'd7: begin
                        if (pal_sel) pal[pal_index(v[4:0])] <= reg_din[5:0];
                        v <= v + addr_step;
                    end
                    default: ;
                endcase
            end

            if (reg_re) begin
                case (reg_addr)
                    3'd2: begin
                        st_vblank <= 1'b0;
                        w         <= 1'b0;
                    end
                    3'd7: begin
                        if (!rendering_now) fill_buffer <= 1'b1;
                        v <= v + addr_step;
                    end
                    default: ;
                endcase
            end
        end
    end

    // -----------------------------------------------------------------
    // What a read of the eight registers gives back
    // -----------------------------------------------------------------
    always @* begin
        case (reg_addr)
            3'd2:    reg_dout = {st_vblank, st_sprite0, st_overflow, 5'd0};
            3'd4:    reg_dout = oam_q;
            3'd7:    reg_dout = pal_sel ? {2'd0, pal_read} : read_buffer;
            default: reg_dout = 8'd0;
        endcase
    end
endmodule
