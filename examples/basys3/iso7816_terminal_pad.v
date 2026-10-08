// The board-level wrapper for `iso7816_terminal`: one bidirectional ball.
//
// Tristate lives here and nowhere else. The core takes `io_i` and drives
// `io_oe`/`io_o`, which is what lets a testbench put two devices on one
// open-drain wire — `reticle sim` refuses a net driven from more than one
// place, so a shared bus has to be modelled as the wired-AND a pull-up
// physically is, and that needs the enable and the value separately.
//
// **Open drain**: the line is pulled low or released, never driven high.
// `io_o` is a constant zero inside the core, so `io_oe` alone decides, and
// the external 1k pull-up to 3.3 V provides the high level. That is why the
// card and this board can share the wire with no contention, and why the
// internal pull-up must stay off in the constraints.
//
// This file is the one `examples/basys3/iso7816_terminal.rcf` names as the
// top; `iso7816_terminal_tb.v` instantiates the core directly.
module iso7816_terminal_pad #(
    parameter CARD_DIV        = 14,
    parameter ETU_CYCLES      = 372,
    parameter FAST_ETU_CYCLES = 4,
    parameter RST_HOLD        = 400,
    parameter VCC_BITS        = 20,
    parameter GAP_ETU         = 256,
    parameter HOST_DIV        = 972,
    parameter WDOG_BITS       = 31
) (
    input  wire        clk,
    output wire        clk_card,
    output wire        rst_card,
    output wire        vcc_en,
    inout  wire        io,
    input  wire        uart_rx_pin,
    output wire        uart_tx_pin,
    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    wire io_oe, io_o;

    // **The PLL lives here, because it can only live at the top.**
    // `clock_mhz` and `clock_locked` are requests to the backend and are
    // honoured on the top module's nets only; inside a submodule the build
    // fails with the net driven by nothing.
    //
    // 112 MHz is chosen so every card clock is an exact, even integer
    // division of it: /14 is 8 MHz, /32 is 3.5, /112 is 1, all at 50 % duty.
    // The solver delivers it at +0.0 ppm.
    (* clock_mhz = 112 *) wire clk_pll;
    (* clock_locked = "clk_pll" *) wire locked;

    iso7816_terminal #(
        .CARD_DIV(CARD_DIV), .ETU_CYCLES(ETU_CYCLES),
        .FAST_ETU_CYCLES(FAST_ETU_CYCLES), .RST_HOLD(RST_HOLD),
        .VCC_BITS(VCC_BITS), .GAP_ETU(GAP_ETU), .HOST_DIV(HOST_DIV),
        .WDOG_BITS(WDOG_BITS)
    ) core (
        .clk(clk_pll), .locked(locked), .clk_card(clk_card), .rst_card(rst_card),
        .vcc_en(vcc_en),
        .io_i(io), .io_oe(io_oe), .io_o(io_o),
        .uart_rx_pin(uart_rx_pin), .uart_tx_pin(uart_tx_pin),
        .led(led), .seg(seg), .dp(dp), .an(an));

    assign io = io_oe ? io_o : 1'bz;
endmodule
