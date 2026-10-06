// vga_out — VGA video output: a raster, colour to a resistor ladder, and
// the two sync pins.
//
// What it does
//   Produces the analogue video signal a DE-15 socket carries, for one
//   of the three modes `video_timing` knows. It is `dvi_tx` with
//   everything that makes DVI taken out: no 8b/10b encoding, no running
//   disparity, no 10:1 serialiser and no double-data-rate output. What
//   is left is the raster, the colour bits driven straight at the
//   board's resistor ladder, and hsync and vsync at the polarity the
//   mode wants — which is the whole of VGA.
//
//   The raster is `video_timing` itself, instantiated and not copied;
//   the package depends on `dvi_tx` for it. So the timings, the sync
//   polarities and the meaning of `de`, `x`, `y` and `frame` are one
//   statement in one file, shared with the DVI path.
//
//   The user side is `dvi_tx`'s, port for port: `x`, `y` and `de` name
//   the pixel being fetched and the colour for it is expected on `r`,
//   `g`, `b` — eight bits a channel — by the next pixel. A design can
//   swap one block for the other without rewiring its pattern
//   generator. The one difference is the clock: `dvi_tx` runs at five
//   times the pixel rate and hands out a `pix_en` it makes itself,
//   while VGA needs no faster clock at all, so this block takes `clk`
//   and `pix_en` and lets the board decide what they are. A design
//   whose clock *is* the pixel clock ties `pix_en` high.
//
//   BPC is how many bits a channel the board's ladder has: four on a
//   Digilent Basys 3, six on many others, eight on a part with a real
//   DAC. The eight bits in are **truncated** to the top BPC, not
//   rounded. Truncation costs nothing, it is monotonic, and it keeps
//   both ends of the range exact: 8'h00 is black and 8'hFF is every
//   output bit set, which is full scale on the ladder. Rounding would
//   need a saturating add to stop 8'hFF carrying out of the field, and
//   would buy half a step in the middle that no resistor ladder's
//   tolerance can show.
//
//   Outside the active area every colour bit is driven low. That is not
//   tidiness: a monitor takes its black level from the back porch, and
//   colour driven during blanking makes it lose sync or float that
//   level until the picture rolls. `de` from the raster is the gate.
//
//   The five pins are registered, and that costs one pixel of latency:
//   the colour, hsync and vsync a pin carries during pixel N+1 are the
//   ones the raster asked for at pixel N. All five move together, so
//   the picture is shifted one pixel and nothing is skewed against
//   anything else. `de`, `x`, `y` and `frame` are the raster's own and
//   are *not* delayed, because they are the fetch interface rather than
//   the signal.
//
// What it does not do
//   No DAC. The colour pins are ordinary LVCMOS outputs and the board
//   is expected to weight and sum them: a Basys 3 has a four-bit R-2R
//   ladder per channel into a 75 ohm load. A board with no ladder gets
//   one bit per channel and eight colours.
//
//   No sync-on-green, no composite sync, no interlace, and none of the
//   modes `video_timing` does not have.
//
//   Nothing here makes the pixel clock. 640x480 at 60 Hz wants 25.175
//   MHz; a board with a 100 MHz oscillator and a two-bit divider gets
//   25.000 MHz, which is 0.7 % slow — 59.6 Hz — and monitors take it. A
//   board that wants it exact asks its PLL, the way `dvi_tx_pll` does.
module vga_out #(
    parameter MODE = 0,
    // Bits per colour channel on the board's ladder.
    parameter BPC  = 4
) (
    input  wire            clk,
    input  wire            rst_n,
    // One pixel per edge where this is high. Tie it high on a clock
    // that is already the pixel clock.
    input  wire            pix_en,

    // The pixel being fetched, and the colour for it. `dvi_tx`'s ports,
    // with the same meaning.
    output wire            de,
    output wire            frame,
    output wire [11:0]     x,
    output wire [11:0]     y,
    input  wire [7:0]      r,
    input  wire [7:0]      g,
    input  wire [7:0]      b,

    // The socket: BPC bits a channel into the ladder, and the two syncs
    // at the polarity MODE wants.
    output wire [BPC-1:0]  vga_r,
    output wire [BPC-1:0]  vga_g,
    output wire [BPC-1:0]  vga_b,
    output wire            vga_hsync,
    output wire            vga_vsync
);
    // The level a sync pin sits at when it is not pulsing, which is the
    // opposite of `video_timing`'s SYNC_POS: mode 0 pulses low, modes 1
    // and 2 pulse high. It is written out once here because a reset
    // value has to be a constant, and
    // `vga_out_syncs_idle_at_the_polarity_of_the_mode` asserts that the
    // pin sits at this level in reset *and* through the back porch, so
    // the two statements cannot drift apart unnoticed.
    localparam SYNC_IDLE = (MODE == 0) ? 1'b1 : 1'b0;

    wire hsync_raw;
    wire vsync_raw;

    video_timing #(.MODE(MODE)) u_timing (
        .clk   (clk),
        .rst_n (rst_n),
        .en    (pix_en),
        .de    (de),
        .hsync (hsync_raw),
        .vsync (vsync_raw),
        .frame (frame),
        .x     (x),
        .y     (y)
    );

    reg [BPC-1:0] out_r;
    reg [BPC-1:0] out_g;
    reg [BPC-1:0] out_b;
    reg           out_hs;
    reg           out_vs;

    assign vga_r     = out_r;
    assign vga_g     = out_g;
    assign vga_b     = out_b;
    assign vga_hsync = out_hs;
    assign vga_vsync = out_vs;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            out_r  <= {BPC{1'b0}};
            out_g  <= {BPC{1'b0}};
            out_b  <= {BPC{1'b0}};
            out_hs <= SYNC_IDLE;
            out_vs <= SYNC_IDLE;
        end else if (pix_en) begin
            out_r  <= de ? r[7 -: BPC] : {BPC{1'b0}};
            out_g  <= de ? g[7 -: BPC] : {BPC{1'b0}};
            out_b  <= de ? b[7 -: BPC] : {BPC{1'b0}};
            out_hs <= hsync_raw;
            out_vs <= vsync_raw;
        end
    end
endmodule
