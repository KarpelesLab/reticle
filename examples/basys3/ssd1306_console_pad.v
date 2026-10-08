// The board-level wrapper for `ssd1306_console`: the serial port, the
// power-on reset, and the two button lines released or driven.
//
// Tristate lives here and nowhere else, for the same reason it does in
// `iso7816_terminal_pad.v`: `reticle sim` resolves two drivers on one net
// but has no pull-up, so a released line reads `z` rather than its idle
// level — a testbench that wants to model the far side has to see the
// request, not the ball. The core therefore asks with `press_left` and
// `press_right` and this file decides what the ball does with that.
//
// **Each line is released except while pressed.** That is what keeps the
// far side's own buttons working and guarantees nothing is ever fought.
// Each has its own 10k resistor defining its idle level, so a press means
// overpowering that resistor in one direction only — and the two
// directions differ:
//
//   `btn_left`   idles LOW through a pull-down  -> press drives HIGH
//   `btn_right`  idles HIGH through a pull-up   -> press drives LOW
//
// A push-pull output would hold each line at its idle level and fight
// anybody pressing the real button, which with a switch to the opposite
// rail is a short limited only by the pin.
//
// **No PLL here.** This design runs straight off the board's 100 MHz
// oscillator, which is why `CLK_DIV` defaults to 868; `iso_display.v` needs
// 112 MHz for the card clock and divides 972 instead.
//
// This file is the one `examples/basys3/ssd1306_console.rcf` names as the
// top; `ssd1306_console_tb.v` instantiates the core directly.
module ssd1306_console_pad #(
    parameter CLK_DIV    = 868,   // 100 MHz / 115200
    parameter TICK_BIT   = 25,
    parameter COLUMNS    = 128,
    parameter PAGES      = 8,
    parameter PRESS_BITS = 23,
    parameter BLOCK_RAM  = 1
) (
    input  wire        clk,

    // Pmod JA, pins 1 to 4.
    input  wire        sclk,
    input  wire        mosi,
    input  wire        dc,
    input  wire        cs_n,

    input  wire        uart_rx_pin,
    output wire        uart_tx_pin,

    inout  wire        btn_left,
    inout  wire        btn_right,

    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    // Saturating reset: ones shift in, so it cannot un-reset. A one-hot
    // walking under an enable can walk off the end, which this directory
    // has already paid for once on hardware.
    reg [15:0] por = 16'h0000;
    always @(posedge clk) por <= {por[14:0], 1'b1};
    wire rst_n = por[15];

    wire [7:0] cmd_data, out_data;
    wire       cmd_valid, out_valid, out_ready;

    uart #(.CLK_DIV(CLK_DIV)) port (
        .clk(clk), .rst_n(rst_n), .div(16'd0),
        .tx_data(out_data), .tx_valid(out_valid), .tx_ready(out_ready),
        .tx(uart_tx_pin),
        .rx(uart_rx_pin), .rx_data(cmd_data), .rx_valid(cmd_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

    wire press_left, press_right;

    ssd1306_console #(
        .TICK_BIT(TICK_BIT), .COLUMNS(COLUMNS), .PAGES(PAGES),
        .PRESS_BITS(PRESS_BITS), .BLOCK_RAM(BLOCK_RAM)
    ) core (
        .clk(clk), .rst_n(rst_n),
        .sclk(sclk), .mosi(mosi), .dc(dc), .cs_n(cs_n),
        .cmd_valid(cmd_valid), .cmd_data(cmd_data),
        .out_valid(out_valid), .out_data(out_data), .out_ready(out_ready),
        .press_left(press_left), .press_right(press_right),
        .led(led), .seg(seg), .dp(dp), .an(an));

    // Driven in one direction or released. Never driven to the idle level.
    assign btn_left  = press_left  ? 1'b1 : 1'bz;
    assign btn_right = press_right ? 1'b0 : 1'bz;
endmodule
