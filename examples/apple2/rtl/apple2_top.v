// apple2_top — the machine on a board: a PLL, a DVI transmitter and
// `apple2`.
//
// This is the top a bitstream is built from. It has almost nothing in it,
// because everything that is the computer is in `apple2` and everything
// that is the video signal is in `dvi_tx` from the IP library. What is
// left is the three things only a board needs:
//
//   * the pixel clock. `clk_x5` is declared, used and left undriven with
//     a `clock_mhz` attribute on it, which is how a design asks Reticle's
//     FPGA flow for a PLL (docs/fpga.md). 126 MHz is five times the
//     640x480 pixel clock of 25.175 MHz rounded to a whole megahertz,
//     0.1 % fast, which is well inside what a monitor accepts. The board
//     says what `clk_ref` is with a `create_clock` in board/*.rcf.
//
//   * the reset. A counter holds `rst_n` low for the first 32768 clocks —
//     about 260 microseconds at 126 MHz, which is longer than the PLL
//     takes to lock — and then releases it for good. An FPGA starts every
//     flip-flop at zero when it is configured, so this is a power-on
//     reset and the board needs no reset pin. Releasing it starts the
//     6502's own seven-cycle RES sequence, which ends by jumping through
//     $FFFC.
//
//   * the pins.
//
// **This module cannot be simulated**, for the same reason `dvi_tx_pll`
// cannot: nothing in the source drives `clk_x5`, because the PLL that
// does is instantiated by the FPGA flow. `tb/apple2_tb.v` simulates
// `apple2` against a bare `video_timing` instead, which is the same
// machine with the serialisers taken off the end; README.md says what
// that does and does not cover.
module apple2_top #(
    parameter ROM_FILE  = "sw/monitor.hex",
    parameter FONT_FILE = "sw/font.hex",
    // 126 MHz / 115200 baud, for the terminal that is the keyboard.
    parameter CLK_DIV   = 1094
) (
    // The board's oscillator. board/*.rcf says how fast it is.
    input  wire       clk_ref,

    // The terminal: the machine transmits on `uart_tx` and its keyboard
    // is whatever arrives on `uart_rx`.
    output wire       uart_tx,
    input  wire       uart_rx,
    // $C030 toggles this.
    output wire       speaker,

    // DVI. Each lane is one pin through a double-data-rate output
    // register on `clk_x5`, two bits a cycle; a family with
    // pseudo-differential outputs makes the pair from one pin in the
    // constraints, and one without needs a second pin driving the
    // complement.
    (* ddr = "clk_x5" *)
    output wire [1:0] tmds_d0,
    (* ddr = "clk_x5" *)
    output wire [1:0] tmds_d1,
    (* ddr = "clk_x5" *)
    output wire [1:0] tmds_d2,
    (* ddr = "clk_x5" *)
    output wire [1:0] tmds_clk
);
    (* clock_mhz = 126 *)
    wire clk_x5;

    reg [15:0] por = 16'd0;
    always @(posedge clk_x5) begin
        if (!por[15]) por <= por + 16'd1;
    end
    wire rst_n = por[15];

    wire        pix_en;
    wire        de;
    wire [11:0] x;
    wire [11:0] y;
    wire [7:0]  r;
    wire [7:0]  g;
    wire [7:0]  b;

    dvi_tx #(
        .MODE     (0)              // 640 x 480 at 60 Hz
    ) u_dvi (
        .clk_x5   (clk_x5),
        .rst_n    (rst_n),
        .pix_en   (pix_en),
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

    apple2 #(
        .ROM_FILE  (ROM_FILE),
        .FONT_FILE (FONT_FILE),
        .CLK_DIV   (CLK_DIV)
    ) u_machine (
        .clk       (clk_x5),
        .rst_n     (rst_n),
        .pix_en    (pix_en),
        .de        (de),
        .x         (x),
        .y         (y),
        .r         (r),
        .g         (g),
        .b         (b),
        .uart_tx   (uart_tx),
        .uart_rx   (uart_rx),
        .speaker   (speaker)
    );
endmodule
