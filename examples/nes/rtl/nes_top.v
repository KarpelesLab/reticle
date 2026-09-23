// nes_top — the whole example: a console, a frame buffer and a DVI
// transmitter, on one clock.
//
// This is the only place the three parts meet, and the only HDL in this
// project that is not either the console board (rtl/nes_console.v) or
// the screen (rtl/nes_video.v). `mos6502`, `ppu2c02` and `dvi_tx` all
// come out of the Reticle IP library through `reticle.proj`.
//
// One clock. `clk_x5` is five times the 640 x 480 pixel clock, which is
// 125.875 MHz and which the board is asked for as 126: that is what
// `dvi_tx` runs on, and a counter divides it down to the console's dot
// rate. Nothing crosses between clocks anywhere in the design, which is
// worth the small inaccuracy it costs:
//
//   * a real console's picture unit runs at 5.369 MHz, one dot per
//     master clock divided by four;
//   * 126 MHz divided by 24 is 5.25 MHz, which is 2.2 % slow, so a
//     frame takes 17.02 ms instead of 16.64 and the console runs at
//     58.8 frames a second rather than 60.1.
//
// Nothing in the console can tell: every part of it is clocked by `en`,
// so its own cycle counts are exactly the ones the tables print. A
// stopwatch could tell, and so could music, which is one more reason
// the sound hardware is not here.
//
// On a board, `clk_x5` would come from the device's PLL rather than
// from a pin — that is what `ip/dvi_tx_pll` is for, and README.md has
// the two-line change. It is a pin here because a design whose clock
// nothing drives cannot be simulated, and this one is simulated hard.
module nes_top #(
    // The cartridge. The paths are relative to wherever the tool that
    // reads them runs; the commands in README.md run from this
    // example's directory.
    parameter PRG_FILE = "sw/demo.hex",
    parameter CHR_FILE = "sw/chr.hex",
    // Vertical mirroring: the two nametables side by side, which is
    // what the demo's sideways scroll needs.
    parameter MIRROR   = 1,
    // Cycles of `clk_x5` per dot of the picture unit.
    parameter DOT_DIV  = 24
) (
    input  wire       clk_x5,

    // The TMDS lanes, two bits per pin per cycle of `clk_x5`.
    (* ddr = "clk_x5" *)
    output wire [1:0] tmds_d0,
    (* ddr = "clk_x5" *)
    output wire [1:0] tmds_d1,
    (* ddr = "clk_x5" *)
    output wire [1:0] tmds_d2,
    (* ddr = "clk_x5" *)
    output wire [1:0] tmds_clk
);
    // -----------------------------------------------------------------
    // Reset: the system resets itself, as `examples/mos6502_computer`
    // does. Every flip-flop starts at zero when the device is
    // configured, so this is a power-on reset on a board and the reset
    // at the start of a simulation. Releasing it starts the processor's
    // own seven-cycle RES sequence, which ends by jumping through
    // $FFFC.
    // -----------------------------------------------------------------
    reg [3:0] por = 4'd0;
    always @(posedge clk_x5) begin
        if (!por[3]) por <= por + 4'd1;
    end
    wire rst_n = por[3];

    // -----------------------------------------------------------------
    // One dot of the picture unit every DOT_DIV cycles.
    // -----------------------------------------------------------------
    reg [4:0] divider;
    always @(posedge clk_x5 or negedge rst_n) begin
        if (!rst_n)                          divider <= 5'd0;
        else if (divider == (DOT_DIV[4:0] - 5'd1)) divider <= 5'd0;
        else                                 divider <= divider + 5'd1;
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
        .clk        (clk_x5),
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
        .clk       (clk_x5),
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

    dvi_tx #(
        // 640 x 480 at 60 Hz.
        .MODE (0)
    ) u_dvi (
        .clk_x5   (clk_x5),
        .rst_n    (rst_n),
        .pix_en   (),
        .de       (de),
        .frame    (),
        .x        (x),
        .y        (y),
        .r        (r),
        .g        (g),
        .b        (b),
        .tmds_d0  (tmds_d0),
        .tmds_d1  (tmds_d1),
        .tmds_d2  (tmds_d2),
        .tmds_clk (tmds_clk)
    );
endmodule
