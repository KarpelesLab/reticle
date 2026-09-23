// apple2_basys3 — the machine on a Digilent Basys 3: a clock divider, a
// VGA output and `apple2`.
//
// This is the second top of the example, and it exists because the first
// one cannot be built for this board. `apple2_top` drives DVI, which
// means TMDS pairs out of double-data-rate registers; the Basys 3 has no
// HDMI and no DVI connector, and `src/fpga/devices/xc7.dev` declares no
// DDR register for the part, so the flow says so in a line and stops:
//
//   port tmds_d0 is not double data rate: `xc7a35t-cpg236` declares
//   neither an IO buffer that registers both edges nor a `ddr_out`
//   register
//
// What the board does have is VGA — twelve bits, four per channel,
// through a resistor ladder to a DE-15 socket — so this top wires the
// same `apple2` to `vga_out` from the IP library instead. Nothing about
// the computer changes: `vga_out` has `dvi_tx`'s fetch interface port
// for port, so `x`, `y`, `de` and the eight-bit colour are wired exactly
// as `apple2_top` wires them.
//
// The three things only a board needs:
//
//   * the pixel clock. The Basys 3 has one oscillator, 100 MHz on W5,
//     and no other clock source. A two-bit divider makes one pixel in
//     four, so the pixel rate is 25.000 MHz against the mode's nominal
//     25.175 — 0.7 % slow, a 59.6 Hz frame — which is inside what a
//     monitor accepts and is what every Basys 3 VGA design does. There
//     is no PLL in this design at all: `dvi_tx` needed one because it
//     runs at five times the pixel rate, and VGA does not.
//
//     Everything runs on the 100 MHz clock with `pix_en` marking the
//     pixels, which is the same arrangement `apple2_top` has at five
//     times the pixel rate. The 6502 gets a bus cycle every
//     CPU_PIX_DIV pixels as before, so at 25 MHz pixels it runs at
//     1.786 MHz against the original's 1.023 and the ECP5 build's
//     1.798.
//
//   * the reset. A counter holds `rst_n` low for the first 32768 clocks,
//     330 microseconds at 100 MHz. An FPGA starts every flip-flop at
//     zero when it is configured, so this is a power-on reset and the
//     board needs no reset pin — which is just as well, because the
//     Basys 3 wires none to a dedicated pin.
//
//   * the pins. board/basys3.rcf names them.
//
// CLK_DIV is 868 and not 1094: the divisor is clocks per serial bit on
// `clk`, and this board's clock is 100 MHz, so 100e6 / 115200 is 868.
// Building this with the ECP5 top's 1094 makes the terminal unreadable.
//
// **This module has not been built by Vivado and has not been loaded
// into a part.** What is proved about it is in tests/apple2.rs: the
// screen it paints read back off its own VGA pins, and the netlist,
// XDC and Tcl script the flow writes for `xc7a35t-cpg236`.
module apple2_basys3 #(
    parameter ROM_FILE  = "sw/monitor.hex",
    parameter FONT_FILE = "sw/font.hex",
    // 100 MHz / 115200 baud, for the terminal that is the keyboard.
    parameter CLK_DIV   = 868
) (
    // The board's single 100 MHz oscillator.
    input  wire       clk,

    // The terminal, through the board's USB-UART bridge.
    output wire       uart_tx,
    input  wire       uart_rx,
    // $C030 toggles this. The Basys 3 has no audio output, so it leaves
    // through a Pmod pin; board/basys3.rcf says which.
    output wire       speaker,

    // VGA: four bits a channel into the board's resistor ladder, and
    // the two syncs.
    output wire [3:0] vga_r,
    output wire [3:0] vga_g,
    output wire [3:0] vga_b,
    output wire       vga_hsync,
    output wire       vga_vsync
);
    reg [15:0] por = 16'd0;
    always @(posedge clk) begin
        if (!por[15]) por <= por + 16'd1;
    end
    wire rst_n = por[15];

    // One pixel in four: 25.000 MHz from the board's 100 MHz.
    reg [1:0] pix_div;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) pix_div <= 2'd0;
        else        pix_div <= pix_div + 2'd1;
    end
    wire pix_en = (pix_div == 2'd3);

    wire        de;
    wire [11:0] x;
    wire [11:0] y;
    wire [7:0]  r;
    wire [7:0]  g;
    wire [7:0]  b;

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

    apple2 #(
        .ROM_FILE  (ROM_FILE),
        .FONT_FILE (FONT_FILE),
        .CLK_DIV   (CLK_DIV)
    ) u_machine (
        .clk       (clk),
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
