// The board-level wrapper for `iso7816_terminal`: one bidirectional ball,
// one PLL and one serial port.
//
// The core takes and gives bytes rather than owning a UART, because
// `iso_display.v` puts it and `ssd1306_console.v` on the board's single
// serial port together. This wrapper is the one-core case of that: the
// whole port belongs to the terminal, so the two streams wire straight
// across with no arbitration.
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
    parameter FAST_ETU_CYCLES = 8,     // with `PPS1 = 97`: 1 Mbaud on the contact
    parameter RST_HOLD        = 400,
    parameter VCC_BITS        = 20,
    parameter GAP_ETU         = 256,
    // 2.000 Mbaud, exactly: 112 MHz / 56, zero error.
    //
    // **Because 115200 cannot keep up with the card.** At F/D = 4 a byte
    // reaches the contact every 6 us, and printed as two hex characters one
    // costs 260 us at 115200 -- forty times too slow, so a card talking at
    // the fast rate was reported with most of it missing. Worse, the card's
    // waiting time at that rate is about 4.8 ms, so a host that must answer
    // could not even be told in time. At 2 Mbaud a byte costs 15 us, a
    // 32-byte response reaches the host in 0.5 ms and an answer goes back
    // in 0.3 ms, which fits inside the card's window with room to spare.
    // The board's FT2232H carries it and Linux has `B2000000`.
    parameter HOST_DIV        = 56,
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

    // The same sixteen-stage shift of `locked` the core resets on, so the
    // port and the core leave reset on the same cycle. Written out twice
    // rather than passed down, because a reset output would be one more
    // port on a core whose reset is private to it; the two are identical
    // logic on the identical input, so they cannot disagree.
    reg [15:0] por = 16'h0000;
    always @(posedge clk_pll) por <= {por[14:0], locked};
    wire rst_n = por[15];

    wire [7:0] cmd_data, out_data;
    wire       cmd_valid, out_valid, out_ready;

    uart #(.CLK_DIV(HOST_DIV)) host (
        .clk(clk_pll), .rst_n(rst_n), .div(16'd0),
        .tx_data(out_data), .tx_valid(out_valid), .tx_ready(out_ready),
        .tx(uart_tx_pin),
        .rx(uart_rx_pin), .rx_data(cmd_data), .rx_valid(cmd_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

    iso7816_terminal #(
        .CARD_DIV(CARD_DIV), .ETU_CYCLES(ETU_CYCLES),
        .FAST_ETU_CYCLES(FAST_ETU_CYCLES), .RST_HOLD(RST_HOLD),
        .VCC_BITS(VCC_BITS), .GAP_ETU(GAP_ETU),
        .WDOG_BITS(WDOG_BITS)
    ) core (
        .clk(clk_pll), .locked(locked), .clk_card(clk_card), .rst_card(rst_card),
        .vcc_en(vcc_en),
        .io_i(io), .io_oe(io_oe), .io_o(io_o),
        .cmd_valid(cmd_valid), .cmd_data(cmd_data), .seph_buttons(2'b00), .buttons_in(2'b00),
        .out_valid(out_valid), .out_data(out_data), .out_ready(out_ready),
        // Nothing else is on this port, so nothing needs holding off a
        // `:` run's characters.
        .hex_run(),
        .led(led), .seg(seg), .dp(dp), .an(an));

    assign io = io_oe ? io_o : 1'bz;
endmodule
