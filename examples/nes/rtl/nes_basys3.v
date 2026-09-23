// nes_basys3 — the console on a Digilent Basys 3: a clock divider, a
// VGA output, and the same console and frame buffer `nes_top` has.
//
// This is the second top of the example, and it exists because the
// first one cannot be built for this board. `nes_top` drives DVI, which
// means TMDS pairs out of double-data-rate registers; the Basys 3 has
// no HDMI and no DVI connector, and `src/fpga/devices/xc7.dev` declares
// no DDR register for the part, so the flow says so in a line and
// stops:
//
//   port tmds_d0 is not double data rate: `xc7a35t-cpg236` declares
//   neither an IO buffer that registers both edges nor a `ddr_out`
//   register
//
// What the board does have is VGA — twelve bits, four per channel,
// through a resistor ladder to a DE-15 socket — so this top wires the
// same `nes_console` and `nes_video` to `vga_out` from the IP library
// instead of `dvi_tx`. Nothing about the console or the frame buffer
// changes: `vga_out` has `dvi_tx`'s fetch interface port for port, so
// `x`, `y`, `de` and the eight-bit colour are wired exactly as
// `nes_top` wires them.
//
// Clocking: two dividers off one oscillator, and no PLL
//
//   The Basys 3 has one clock source, 100 MHz on W5, and nothing else.
//   Everything in this design runs on it, with two enables below it —
//   which is the arrangement `nes_top` already has, only at a different
//   rate:
//
//     clk    100 MHz      vga_out, the frame buffer's read port
//      /4     25.000 MHz  one pixel of the 640 x 480 raster
//      /19     5.263 MHz  one dot of the picture unit
//      /3      1.754 MHz  one cycle of the processor
//
//   * the pixel clock. One pixel in four is 25.000 MHz against the
//     mode's nominal 25.175 — 0.7 % slow, a 59.6 Hz frame — which is
//     inside what a monitor accepts and is what every Basys 3 VGA
//     design does. There is no PLL in this design at all: `dvi_tx`
//     needed one because it runs at five times the pixel rate, and VGA
//     does not.
//
//   * the dot rate. A real picture unit runs at 5.369 MHz, and 100 / 19
//     is 5.263, which is 2.0 % slow — a frame every 16.98 ms, 58.9
//     frames a second rather than 60.1. That is very slightly closer
//     than the ECP5 build's 126 / 24 = 5.25 MHz and 2.2 %. Nothing
//     inside the console can tell either way: every part of it moves on
//     `en` and on nothing else, so its own cycle counts are exactly the
//     ones the tables print. A stopwatch could tell, and so could
//     music, which is one more reason the sound hardware is not here.
//
//     DOT_DIV and the pixel divider are not related to each other and
//     do not need to be: the frame buffer is what sits between the two
//     rasters, and it has always been written by one and read by the
//     other. What that costs is tearing — the console writes a frame
//     every 16.98 ms and the screen reads one every 16.78 ms, so the
//     two edges walk past each other about five times a minute — and
//     what it buys is one clock domain and no synchronisers.
//
//   * the reset. A counter holds `rst_n` low for the first 32768
//     clocks, 330 microseconds at 100 MHz. An FPGA starts every
//     flip-flop at zero when it is configured, so this is a power-on
//     reset and the board needs no reset pin — which is just as well,
//     because the Basys 3 wires none to a dedicated pin.
//
//   * the pins. board/basys3.rcf names them.
//
// Colour: the board renders four bits a channel, not eight. `vga_out`
// truncates, so two of the palette's sixty-four entries that differ
// only below their top four bits come out as one colour here. The
// README says which pair; it is one pair, and the demo does not use it.
//
// **This module has not been built by Vivado and has not been loaded
// into a part.** What is proved about it is in tests/nes.rs: the frame
// it paints read back off its own VGA pins, and the netlist, XDC and
// Tcl script the flow writes for `xc7a35t-cpg236`.
module nes_basys3 #(
    // The cartridge. The paths are relative to wherever the tool that
    // reads them runs; the commands in README.md run from this
    // example's directory.
    parameter PRG_FILE = "sw/demo.hex",
    parameter CHR_FILE = "sw/chr.hex",
    // Vertical mirroring: the two nametables side by side, which is
    // what the demo's sideways scroll needs.
    parameter MIRROR   = 1,
    // Cycles of `clk` per dot of the picture unit: 100 MHz / 19.
    parameter DOT_DIV  = 19
) (
    // The board's single 100 MHz oscillator.
    input  wire       clk,

    // VGA: four bits a channel into the board's resistor ladder, and
    // the two syncs.
    output wire [3:0] vga_r,
    output wire [3:0] vga_g,
    output wire [3:0] vga_b,
    output wire       vga_hsync,
    output wire       vga_vsync
);
    // -----------------------------------------------------------------
    // Power-on reset
    // -----------------------------------------------------------------
    reg [15:0] por = 16'd0;
    always @(posedge clk) begin
        if (!por[15]) por <= por + 16'd1;
    end
    wire rst_n = por[15];

    // -----------------------------------------------------------------
    // One pixel in four: 25.000 MHz from the board's 100 MHz.
    // -----------------------------------------------------------------
    reg [1:0] pix_div;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) pix_div <= 2'd0;
        else        pix_div <= pix_div + 2'd1;
    end
    wire pix_en = (pix_div == 2'd3);

    // -----------------------------------------------------------------
    // One dot of the picture unit every DOT_DIV cycles.
    // -----------------------------------------------------------------
    reg [4:0] divider;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n)                                divider <= 5'd0;
        else if (divider == (DOT_DIV[4:0] - 5'd1)) divider <= 5'd0;
        else                                       divider <= divider + 5'd1;
    end
    wire dot_en = (divider == 5'd0);

    // -----------------------------------------------------------------
    // The console
    // -----------------------------------------------------------------
    wire       vid_de;
    wire [7:0] vid_x;
    wire [7:0] vid_y;
    wire [5:0] vid_color;

    nes_console #(
        .PRG_FILE (PRG_FILE),
        .CHR_FILE (CHR_FILE),
        .MIRROR   (MIRROR)
    ) u_console (
        .clk        (clk),
        .rst_n      (rst_n),
        .en         (dot_en),
        .vid_de     (vid_de),
        .vid_x      (vid_x),
        .vid_y      (vid_y),
        .vid_color  (vid_color),
        .vid_frame  (),
        .dbg_dot    (),
        .dbg_line   (),
        .dbg_pc     (),
        .dbg_retire (),
        .dbg_dma    ()
    );

    // -----------------------------------------------------------------
    // The screen
    // -----------------------------------------------------------------
    wire [11:0] x;
    wire [11:0] y;
    wire        de;
    wire [7:0]  r;
    wire [7:0]  g;
    wire [7:0]  b;

    nes_video u_video (
        .clk       (clk),
        .en        (dot_en),
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
        .MODE      (0),             // 640 x 480 at 60 Hz
        .BPC       (4)              // the board's ladder
    ) u_vga (
        .clk       (clk),
        .rst_n     (rst_n),
        .pix_en    (pix_en),
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
endmodule
