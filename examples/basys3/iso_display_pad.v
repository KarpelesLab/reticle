// The board-level wrapper for `iso_display`: the PLL and the three
// bidirectional balls.
//
// Everything that makes the combined design what it is — one serial port,
// the router, the arbiter, both halves — is in `iso_display.v`. This file
// exists because two things can only live at the top:
//
// **The clock.** `clock_mhz` and `clock_locked` are requests to the backend
// and are honoured on the **top** module's nets only. A PLL request inside a
// submodule fails the build with the clock net driven by nothing, three
// steps from its cause; this directory has paid for that once already.
//
// 112 MHz is chosen so every card clock is an exact, even integer division
// of it: /14 is 8 MHz, /32 is 3.5, /112 is 1, all at 50 % duty. The solver
// delivers it at +0.0 ppm. The display half is indifferent — it only needs
// `sclk` to be slow relative to the system clock, and 112 MHz gives it more
// margin than the 100 it had.
//
// **The tristates.** `reticle sim` resolves two drivers on one net but has
// no pull-up, so a released open-drain line reads `z` there rather than
// high. A testbench with a model card on the contact therefore has to see
// the enable and the value separately, which is why the core takes `io_i`
// and drives `io_oe`/`io_o`, and why it asks for a button press rather than
// driving a ball.
//
//   `io`         open drain: pulled low or released, never driven high.
//                `io_o` is a constant zero inside the card half, so `io_oe`
//                alone decides and the external 1k pull-up to 3.3 V makes
//                the high level. That is why the card and this board can
//                share the wire with no contention, and why the internal
//                pull-up must stay off in the constraints.
//   `btn_left`   idles LOW through the far side's 10k pull-down, so a press
//                drives HIGH and nothing else ever drives it.
//   `btn_right`  idles HIGH through a 10k pull-up, so a press drives LOW.
//
// A push-pull output on either button line would hold it at its idle level
// and fight anybody pressing the real button, which with a switch to the
// opposite rail is a short limited only by the pin.
//
// This file is the one `examples/basys3/iso_display.rcf` names as the top;
// `iso_display_tb.v` instantiates `iso_display` directly.
module iso_display_pad #(
    parameter CARD_DIV        = 14,
    parameter ETU_CYCLES      = 372,
    parameter FAST_ETU_CYCLES = 4,
    parameter RST_HOLD        = 400,
    parameter VCC_BITS        = 20,
    parameter GAP_ETU         = 256,
    parameter WDOG_BITS       = 31,
    parameter HOST_DIV        = 972,
    parameter TICK_BIT        = 25,
    parameter COLUMNS         = 128,
    parameter PAGES           = 8,
    parameter PRESS_BITS      = 23,
    parameter BLOCK_RAM       = 1
) (
    input  wire        clk,

    // The card.
    output wire        clk_card,
    output wire        rst_card,
    output wire        vcc_en,
    inout  wire        io,

    // The display link, Pmod JA 1 to 4.
    input  wire        sclk,
    input  wire        mosi,
    input  wire        dc,
    input  wire        cs_n,

    // The far side's two button lines, Pmod JC 1 and 2.
    inout  wire        btn_left,
    inout  wire        btn_right,

    input  wire        uart_rx_pin,
    output wire        uart_tx_pin,

    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    (* clock_mhz = 112 *) wire clk_pll;
    (* clock_locked = "clk_pll" *) wire locked;

    wire io_oe, io_o;
    wire press_left, press_right;

    iso_display #(
        .CARD_DIV(CARD_DIV), .ETU_CYCLES(ETU_CYCLES),
        .FAST_ETU_CYCLES(FAST_ETU_CYCLES), .RST_HOLD(RST_HOLD),
        .VCC_BITS(VCC_BITS), .GAP_ETU(GAP_ETU), .WDOG_BITS(WDOG_BITS),
        .HOST_DIV(HOST_DIV), .TICK_BIT(TICK_BIT),
        .COLUMNS(COLUMNS), .PAGES(PAGES), .PRESS_BITS(PRESS_BITS),
        .BLOCK_RAM(BLOCK_RAM)
    ) core (
        .clk(clk_pll), .locked(locked),
        .clk_card(clk_card), .rst_card(rst_card), .vcc_en(vcc_en),
        .io_i(io), .io_oe(io_oe), .io_o(io_o),
        .sclk(sclk), .mosi(mosi), .dc(dc), .cs_n(cs_n),
        .press_left(press_left), .press_right(press_right),
        .uart_rx_pin(uart_rx_pin), .uart_tx_pin(uart_tx_pin),
        .led(led), .seg(seg), .dp(dp), .an(an));

    assign io        = io_oe ? io_o : 1'bz;
    assign btn_left  = press_left  ? 1'b1 : 1'bz;
    assign btn_right = press_right ? 1'b0 : 1'bz;
endmodule
