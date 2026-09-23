// nes_vga_tb — the console behind `vga_out`, watched at the pins.
//
// `tb/nes_tb.v` next to this one reads the picture off the console's own
// *video port*, one palette index per dot, and folds it into a hash.
// This one proves the rest of the chain, and proves it at a socket: the
// frame buffer, the doubling onto a 640 x 480 raster, `ppu_palette`, and
// the twelve colour pins and two sync pins a Digilent Basys 3 has. So
// the console is wired to `nes_video` and `vga_out` exactly as
// `rtl/nes_basys3.v` wires them — four bits a channel, as the board's
// resistor ladder takes them — and `tests/nes.rs` decodes `vga_rgb`,
// which is those twelve pins and nothing inside the design.
//
// How it stays cheap. A whole 640 x 480 frame is 420,000 pixel slots,
// and the console needs two of its own frames before the picture is the
// one the tests compare against, so the two jobs are done one after the
// other rather than at once:
//
//   * the console runs at **one dot per clock**, which is the fastest it
//     goes — `nes_console` moves on `en` and on nothing else, so the
//     frames it draws are the same ones `nes_basys3` draws at one dot in
//     nineteen — with the raster held in reset the whole time.
//   * when frame 1 is complete in the frame buffer the console's `en`
//     goes low for good. `vid_frame` marks dot 0 of line 0, where
//     `vid_de` is low, so no pixel of the next frame has been written
//     when it arrives and the buffer is frozen exactly on a frame
//     boundary. Nothing writes to it again, so there is no tearing to
//     reason about.
//   * only then is the raster let go, and it draws **one** frame, which
//     is frame 0 of the raster: no run is spent looking for a frame
//     boundary and none is spent drawing a frame nobody reads.
//
// One pixel per clock rather than the one in four `nes_basys3` runs at
// costs the picture one pixel of delay that the board does not have.
// `nes_video` reads the frame buffer through a register, and on the
// board that register settles inside the four clocks between one pixel
// and the next; here there is one clock between pixels, so the colour a
// pin carries is the one the raster asked for **two** pixels earlier —
// one for the frame buffer's read register and one for `vga_out`'s
// output register. `tests/nes.rs` says the same number in its own words
// and shifts by it. Nothing else about the picture changes: the border
// is 64 pixels wide on either side, so no part of the console's picture
// is near enough to the edge of the active area for the shift to push
// it out.
`timescale 1ns / 1ns

module nes_vga_tb;
    // One pixel per clock. 20 ns is 25 MHz, which is the pixel rate a
    // Basys 3 makes by dividing its 100 MHz oscillator by four.
    localparam HALF    = 20;
    // The frame of the demo the picture is taken from, counted the way
    // `tb/nes_tb.v` counts: the first `vid_frame` is the one left over
    // from reset, the second starts the frame this records.
    localparam FRAME   = 2;
    // The last pixel and the last line of the 640 x 480 raster, which is
    // as far as the frame has to be drawn: the two lines of vertical
    // sync are at 490 and 491, below the picture.
    localparam H_LAST  = 799;
    localparam V_LAST  = 524;
    // The line after the last one of the picture, which for this design
    // is the whole active area: 240 console lines doubled is 480.
    localparam PIC_END = 480;
    // Long enough for the whole run with room to spare; reaching it
    // means something has stopped.
    localparam TIMEOUT = 4000000 * HALF;

    reg clk = 1'b0;
    always #HALF clk = ~clk;

    reg rst_n = 1'b0;
    // The raster's own reset, held while the console draws.
    reg vid_rst_n = 1'b0;

    // -----------------------------------------------------------------
    // The console
    // -----------------------------------------------------------------
    wire       vid_de;
    wire [7:0] vid_x;
    wire [7:0] vid_y;
    wire [5:0] vid_color;
    wire       vid_frame;

    // High while the console is drawing, low once the frame buffer holds
    // the frame this testbench is for.
    reg        con_en = 1'b1;
    reg [1:0]  frames = 2'd0;

    always @(posedge clk) begin
        if (!rst_n) begin
            frames <= 2'd0;
            con_en <= 1'b1;
        end else if (vid_frame) begin
            if (frames == FRAME) con_en <= 1'b0;
            else                 frames <= frames + 2'd1;
        end
    end

    nes_console u_console (
        .clk        (clk),
        .rst_n      (rst_n),
        .en         (con_en),
        .vid_de     (vid_de),
        .vid_x      (vid_x),
        .vid_y      (vid_y),
        .vid_color  (vid_color),
        .vid_frame  (vid_frame),
        .dbg_dot    (),
        .dbg_line   (),
        .dbg_pc     (),
        .dbg_retire (),
        .dbg_dma    ()
    );

    // -----------------------------------------------------------------
    // The screen, pins and all
    // -----------------------------------------------------------------
    wire        de;
    wire [11:0] x;
    wire [11:0] y;
    wire [7:0]  r;
    wire [7:0]  g;
    wire [7:0]  b;
    wire [3:0]  vga_r;
    wire [3:0]  vga_g;
    wire [3:0]  vga_b;
    wire        vga_hsync;
    wire        vga_vsync;

    nes_video u_video (
        .clk       (clk),
        .en        (con_en),
        .vid_de    (vid_de),
        .vid_x     (vid_x),
        .vid_y     (vid_y),
        .vid_color (vid_color),
        .x         (x),
        .y         (y),
        .de        (de),
        .r         (r),
        .g         (g),
        .b         (b)
    );

    vga_out #(
        .MODE      (0),
        .BPC       (4)
    ) u_vga (
        .clk       (clk),
        .rst_n     (vid_rst_n),
        .pix_en    (1'b1),
        .de        (de),
        .frame     (),
        .x         (x),
        .y         (y),
        .r         (r),
        .g         (g),
        .b         (b),
        .vga_r     (vga_r),
        .vga_g     (vga_g),
        .vga_b     (vga_b),
        .vga_hsync (vga_hsync),
        .vga_vsync (vga_vsync)
    );

    // The twelve colour pins as one word, which is what the test
    // samples. Nothing inside the design is watched.
    wire [11:0] vga_rgb = {vga_r, vga_g, vga_b};

    // -----------------------------------------------------------------
    // The run
    // -----------------------------------------------------------------
    initial begin
        repeat (8) @(posedge clk);
        rst_n = 1'b1;

        // The console draws until the frame buffer holds the frame this
        // testbench is for, and then stops for good.
        wait (con_en == 1'b0);
        $display("nes_vga_tb: the frame buffer is frozen at %0t", $time);

        // Now the raster, from the top of its own frame 0 — all 525
        // lines and not just the 480 the picture ends on, because the
        // two lines of vertical sync are at 490 and the test reads them
        // off the pin. The last few clocks cover the two pixels of
        // output latency several times over.
        @(posedge clk);
        vid_rst_n = 1'b1;
        wait (y == PIC_END);
        $display("nes_vga_tb: the picture is drawn at %0t", $time);
        wait (y == V_LAST);
        wait (x == H_LAST);
        repeat (4) @(posedge clk);
        $display("nes_vga_tb: frame 0 recorded at %0t", $time);
        $finish;
    end

    initial begin
        #(TIMEOUT);
        $display("nes_vga_tb: timed out with con_en %b at y %0d", con_en, y);
        $finish;
    end
endmodule
