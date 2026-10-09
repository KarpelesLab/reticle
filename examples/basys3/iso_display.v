// Both halves of a working system in one bitstream: an ISO 7816 card
// brought up and talked to, and a 128x64 SSD1306 pretending to be a panel —
// on one serial port.
//
// ===================================================================
// WHY ONE BITSTREAM
// ===================================================================
//
// The device this drives has to be initialised over its ISO 7816 contact
// and then immediately starts driving SPI at the display. Two bitstreams
// cannot do that: reloading the part drops every output, `vcc_en` with them,
// which opens the PhotoMOS relay and power-cycles the device — so whatever
// the first bitstream negotiated is gone before the second one starts. The
// two halves therefore have to be resident together, and the only thing
// that stopped them being so is that each owned a `uart` and the board has
// one serial port.
//
// So neither owns one now. `iso7816_terminal.v` and `ssd1306_console.v`
// take a received-byte stream and give a transmit stream, this module has
// the single `uart`, and a router and an arbiter sit between.
//
// **The PLL is not here, it is in `iso_display_pad.v`.** `clock_mhz` and
// `clock_locked` are requests to the backend and are honoured on the **top**
// module's nets only — inside a submodule the build fails with the clock net
// driven by nothing, which is a symptom three steps from its cause. The pad
// also holds the three tristates. That split is what makes this module
// testable: `reticle sim` resolves two drivers on one net but has no
// pull-up, so a released open-drain line reads `z` there rather than high,
// and a testbench that wants a model card on the contact has to see
// `io_oe`/`io_o` rather than a ball. `iso_display_tb.v` drives this module
// and `examples/basys3/iso_display.rcf` names the pad as the top.
//
// ===================================================================
// WIRING
// ===================================================================
//
// The union of the two halves' pins and nothing else.
//
//   JXADC1  J3   io         the card's single line, open drain, external
//                           1k pull-up to 3.3 V
//   JXADC2  L3   clk_card   112 MHz / CARD_DIV; /14 is exactly 8 MHz
//   JXADC3  M2   rst_card   active low
//   JXADC4  N2   vcc_en     an AQV210 PhotoMOS relay's LED, through a
//                           series resistor. **This switches the user's
//                           device's 3.3 V supply.** Nothing in the
//                           display half can reach it: it comes out of the
//                           ISO core's activation sequence and out of
//                           nothing else, and no display command is an ISO
//                           command (see the alphabet rule below).
//   JXADC5  —    GND
//
//   JA1     J1   sclk       the display master's clock
//   JA2     L2   mosi       its data: commands and pixels on one wire
//   JA3     J2   dc         low for a command byte, high for a pixel byte
//   JA4     G2   cs_n       low while a byte is on the wire
//   JA5     —    GND
//
//   JC1     K17  btn_left   the far side's LEFT button line, released
//                           except while pressed
//   JC2     M18  btn_right  the same for RIGHT
//
//   B18/A18      the board's USB-UART bridge, 2 Mbaud 8N1 (`HOST_DIV`)
//
// ===================================================================
// THE CONVERSATION
// ===================================================================
//
// One port, two command sets, no collisions. The **card** half keeps every
// letter it had, because they are documented, measured against a real
// device and learned by a person at a terminal:
//
//   A          activate: power, clock, hold reset 400 card clocks, release
//   P          send PPS `FF 10 97 78`; the echo comes back as hex and the
//              **host** compares it
//   F          switch to the fast rate            S   switch back
//   :<hex>     send those bytes to the card. `:00A4040C` sends four. Hex
//              until anything that is not a hex digit ends the run.
//   L          drive the contact low for 18 ms and read it back: `+PAD`
//   C          the same with the pad's data input left on the constant leg
//   D          deactivate: reset, clock, line, then power — in that order
//   s          status, 32 hex characters        k   a known constant
//
// The **display** half's letters moved, and `s d a q r L R` are what they
// were:
//
//   ?          status: one line of 32 hex characters
//   g          get the frame buffer: 8 lines of 256 hex characters
//   o          dump after every completed frame        n   stop
//   z          zero the SPI receiver and the display. It answers nothing;
//              ask with `?`.
//   <          press the far side's LEFT button for 2**PRESS_BITS clocks
//   >          the same for its RIGHT button
//
// **The alphabet rule.** No display command is a hexadecimal digit and none
// is an ISO command. The first half of that is what makes a `:` run safe:
// every character inside one is a hex digit, so a run of card bytes can
// never contain a display command. The second half is what makes the
// router trivial — every received byte goes to **both** cores, and each
// recognises only its own and is silent about the rest.
//
// A `:` run is held off the display half outright, by the card half's own
// `hex_run` bit: every character of a run, including the one that
// terminates it, is the card's. That is one bit read in two places rather
// than a second copy of the run's state, which is the only reason it is
// here and not re-derived — two copies of that state could disagree.
//
// End a run with CR, LF or a space all the same: the terminating character
// does nothing in either half, so a run closed with `g` would need a second
// `g` to get a dump.
//
// An unrecognised character is now silence from both halves. The display
// half used to answer anything it did not know with a status line, which
// would have made every `A`, every `P` and every line ending a host sends
// produce one.
//
// Both halves see the other's commands, which has one consequence worth
// knowing: the card half's idle watchdog is reloaded by any received byte,
// so working the display keeps the card activated. On its own the card half
// would have deactivated after `2**WDOG_BITS` idle clocks, nineteen seconds
// at 112 MHz.
//
// ===================================================================
// HOW THE PORT IS SHARED
// ===================================================================
//
// **A line at a time, and never half a character.** Both halves emit
// CRLF-terminated lines. The arbiter grants the port to one half, and gives
// it up only when the port has accepted an **LF** from that half. While a
// half is granted, the other's `out_ready` is low, so its character waits —
// a core holds `out_valid` and `out_data` until a cycle in which its ready
// is high, so waiting costs nothing and loses nothing.
//
// **Why that cannot corrupt a line.** `0x0A` appears in neither half's
// output except as the second half of a line ending: the card half emits
// hex digits, `+`, `-`, `>`, `!`, the banner letters and CR, and the display
// half emits hex digits and CR. So "the port accepted an LF from the owner"
// is exactly "the owner finished a line", and between the first character of
// a line and its LF no character from the other half can be accepted,
// because its ready is held low for that whole time.
//
// **Why it cannot deadlock.** Every line either ends on its own or ends
// because the contact went quiet: the display half's status line and each
// page of a dump run to completion out of registers, the card half's
// banners and status line likewise, and its stream of card bytes is one
// line that the `GAP_ETU` idle timer closes with a CRLF whenever the card
// stops talking. A card that talks for ever is genuinely one unfinished
// line, and blocking on it is the right answer.
//
// **Round robin, not priority.** Whichever half finished the previous line
// loses the next tie. Strict priority for the card half was the first
// design, and it can starve a dump for as long as a card keeps chattering;
// alternating cannot, and it costs the card half nothing it was not already
// going to lose — see below.
//
// **What a shared port does lose.** The card half holds exactly one
// received byte: a second one arriving before the first has been printed
// overwrites it. One character at 115200 baud is 87 us, a dump page line is
// 256 characters and so about 22 ms, and a card at 2 Mbaud can produce a
// byte every 6 us. So **asking for a dump while the card is talking loses
// card bytes** — as does asking for anything else, and as the card half on
// its own always did, since 11.5 kB/s of serial port cannot carry 250 kB/s
// of card. The normal order of work does not collide: initialise and talk
// to the card, then drive the display and read it back.
//
// One consequence of line-at-a-time sharing is worth knowing: if the card
// is talking while a dump goes out, the dump's eight page lines may have
// card lines between them. Both are hex, and a host that cares must either
// dump while the contact is quiet or tell them apart by length — a page
// line is exactly `2 * COLUMNS` characters.
//
// ===================================================================
// WHAT THE BOARD SHOWS WITHOUT A HOST
// ===================================================================
//
//   LD14   heartbeat, from the card half
//   LD13   a parity error has been seen on the contact
//   LD12   the contact is at the fast rate
//   LD11   the PLL is locked
//   LD10   rst_card is released
//   LD9    the card clock is running
//   LD8    **vcc_en: the relay is closed and the device is powered**
//   LD7    heartbeat, from the display half
//   LD6    a frame was dropped because a dump was in progress
//   LD5    a command the display half does not implement
//   LD4    the SPI receiver has seen a bit error    should stay dark
//   LD3    dumping after every frame, from `o`
//   LD2    a dump is going out
//   LD1    the display has been told it is on
//   LD0    the serial port is granted to the display half
//
// LD6 is skipped on the silkscreen — ball U14 is in a `LIOB33_SING` tile,
// so `led[5]` is LD5 and `led[6]` is LD7. The flow reaches that tile now
// (`fpga::xray::TileAlias`); the numbering is kept because it is what a
// person reads off the board.
//
// The four digits alternate every `2**SHOW_BIT` clocks, about 0.3 s at
// 112 MHz: first the card bytes received, then the frames the display has
// completed, each in hexadecimal. **The decimal point is lit while the
// frame count is showing**, which is the only way to tell the two apart.
//
// ===================================================================
// WHAT IS CHECKED
// ===================================================================
//
// `iso_display_tb.v` drives both halves in one simulation: the activation
// sequence, a real card's 14-byte ATR, the PPS exchange, the rate change,
// a byte to the card at the fast rate, and SSD1306 traffic driven bit by
// bit into the four pins and dumped back byte for byte — with both halves
// interleaving on the one port throughout.
//
// **No part has run this.** Each half separately has: the card half's
// activation, framing, ATR, PPS and rate change are measured on the owner's
// own device, and the display half has never been driven by a real master
// at all. What this file adds over those two is checked in simulation only.
module iso_display #(
    // ---- The card half. `iso7816_terminal.v` explains every one. ----
    parameter CARD_DIV        = 14,
    parameter ETU_CYCLES      = 372,
    parameter FAST_ETU_CYCLES = 8,
    parameter RST_HOLD        = 400,
    parameter VCC_BITS        = 20,
    parameter GAP_ETU         = 256,
    parameter WDOG_BITS       = 31,
    // ---- The shared serial port. 972 is 112 MHz / 115200. ----
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
    // ---- The display half. ----
    parameter TICK_BIT        = 25,
    parameter COLUMNS         = 128,
    parameter PAGES           = 8,
    parameter PRESS_BITS      = 23,
    parameter BLOCK_RAM       = 1,
    // Which bit of a free-running counter picks what the digits show. 25
    // is about 0.3 s at 112 MHz.
    parameter SHOW_BIT        = 25
) (
    // 112 MHz from the pad's PLL, and its lock.
    input  wire        clk,
    input  wire        locked,

    // ---- The card ----
    output wire        clk_card,
    output wire        rst_card,
    output wire        vcc_en,
    input  wire        io_i,
    output wire        io_oe,
    output wire        io_o,

    // ---- The display link, Pmod JA 1 to 4 ----
    input  wire        sclk,
    input  wire        mosi,
    input  wire        dc,
    input  wire        cs_n,

    // ---- A request to press one of the far side's buttons ----
    output wire        press_left,
    output wire        press_right,

    // ---- The one serial port ----
    input  wire        uart_rx_pin,
    output wire        uart_tx_pin,

    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    // The same sixteen-stage shift of `locked` that `iso7816_terminal`
    // builds privately, so the port, the display half and the card half all
    // leave reset on the same cycle: identical logic on an identical input
    // cannot disagree. Ones shift in behind `locked`, so it cannot
    // un-reset once the clock is trustworthy.
    reg [15:0] por = 16'h0000;
    always @(posedge clk) por <= {por[14:0], locked};
    wire rst_n = por[15];

    // =================================================================
    // The one serial port
    // =================================================================

    wire [7:0] cmd_data;
    wire       cmd_valid;

    wire [7:0] tx_data;
    wire       tx_valid, tx_ready;

    uart #(.CLK_DIV(HOST_DIV)) host (
        .clk(clk), .rst_n(rst_n), .div(16'd0),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
        .tx(uart_tx_pin),
        .rx(uart_rx_pin), .rx_data(cmd_data), .rx_valid(cmd_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

    // =================================================================
    // The two halves
    // =================================================================
    //
    // **The router is a broadcast and one gate.** Every received byte goes
    // to both cores, because the alphabets are disjoint: each acts on its
    // own letters and is silent about the other's.
    //
    // The gate is the card half's `hex_run`, high while a `:` run of bytes
    // for the card is being read. Those characters are data, and the card
    // half holds them off its own command decoders with that same bit, so
    // the display half is held off with it too — one bit, read in two
    // places, rather than a second copy of the state that could disagree
    // with it. It covers the run's terminating character as well, which is
    // still inside the run on the cycle it arrives, so ending a run with
    // `g` or `>` cannot do the display thing by accident.
    //
    // The alphabet rule is not made redundant by that gate: it is what
    // makes a run's hexadecimal unable to be a display command in the first
    // place, and what keeps every character outside a run unambiguous.
    //
    // **The card half sees the display half's commands too**, and one thing
    // follows from that: its idle watchdog is reloaded by any received byte,
    // so asking the display for a dump also keeps the card activated. That
    // is the behaviour worth having — the watchdog is there to drop a card
    // nobody is talking to, and a person working the display is still at the
    // terminal — but it is a change from the card half on its own, where
    // only its own commands could do it.

    wire [7:0]  iso_out_data;
    wire        iso_out_valid, iso_out_ready;
    wire        hex_run;
    wire [14:0] iso_led;
    wire [6:0]  iso_seg;
    wire [3:0]  iso_an;

    iso7816_terminal #(
        .CARD_DIV(CARD_DIV), .ETU_CYCLES(ETU_CYCLES),
        .FAST_ETU_CYCLES(FAST_ETU_CYCLES), .RST_HOLD(RST_HOLD),
        .VCC_BITS(VCC_BITS), .GAP_ETU(GAP_ETU), .WDOG_BITS(WDOG_BITS)
    ) card (
        .clk(clk), .locked(locked),
        .clk_card(clk_card), .rst_card(rst_card), .vcc_en(vcc_en),
        .io_i(io_i), .io_oe(io_oe), .io_o(io_o),
        .cmd_valid(cmd_valid), .cmd_data(cmd_data),
        // `<` and `>` press the device's buttons through `W`'s engine as
        // well as on the JC lines: with the FPGA as the MCU, a press is a
        // BUTTON_PUSH_EVENT and not a level on a wire.
        .seph_buttons({press_right, press_left}),
        .out_valid(iso_out_valid), .out_data(iso_out_data),
        .out_ready(iso_out_ready), .hex_run(hex_run),
        .led(iso_led), .seg(iso_seg), .dp(), .an(iso_an));

    wire [7:0]  disp_out_data;
    wire        disp_out_valid, disp_out_ready;
    wire [14:0] disp_led;
    wire [6:0]  disp_seg;
    wire [3:0]  disp_an;

    // The one gate: a `:` run's characters belong to the card.
    wire disp_cmd_valid = cmd_valid & ~hex_run;

    ssd1306_console #(
        .TICK_BIT(TICK_BIT), .COLUMNS(COLUMNS), .PAGES(PAGES),
        .PRESS_BITS(PRESS_BITS), .BLOCK_RAM(BLOCK_RAM)
    ) panel (
        .clk(clk), .rst_n(rst_n),
        .sclk(sclk), .mosi(mosi), .dc(dc), .cs_n(cs_n),
        .cmd_valid(disp_cmd_valid), .cmd_data(cmd_data),
        .out_valid(disp_out_valid), .out_data(disp_out_data),
        .out_ready(disp_out_ready),
        .press_left(press_left), .press_right(press_right),
        .led(disp_led), .seg(disp_seg), .dp(), .an(disp_an));

    // =================================================================
    // The arbiter
    // =================================================================
    //
    // A grant lasts a line: it is taken when the port is free and a half
    // has a character ready, and given up when the port accepts that half's
    // LF. The header argues that this cannot split a line and cannot
    // deadlock; the two properties it rests on are that `0x0A` appears in
    // neither half's output except as the end of a line, and that every
    // line ends.

    localparam [1:0] OWN_NONE = 2'd0, OWN_ISO = 2'd1, OWN_DISP = 2'd2;

    reg [1:0] owner = OWN_NONE;
    // Which half finished the last line, so that it loses the next tie.
    reg       last_iso = 1'b0;

    wire iso_grant  = (owner == OWN_ISO);
    wire disp_grant = (owner == OWN_DISP);

    // Only the owner is ever ready, which is the whole of the back pressure.
    assign iso_out_ready  = iso_grant  & tx_ready;
    assign disp_out_ready = disp_grant & tx_ready;

    assign tx_data  = disp_grant ? disp_out_data : iso_out_data;
    assign tx_valid = (iso_grant  & iso_out_valid)
                    | (disp_grant & disp_out_valid);

    wire accepted = tx_valid & tx_ready;
    wire line_end = accepted & (tx_data == 8'd10);

    always @(posedge clk) begin
        if (!rst_n) begin
            owner    <= OWN_NONE;
            last_iso <= 1'b0;
        end else if (owner == OWN_NONE) begin
            // A tie goes to whoever did not finish the previous line.
            if (iso_out_valid && !(last_iso && disp_out_valid))
                owner <= OWN_ISO;
            else if (disp_out_valid)
                owner <= OWN_DISP;
        end else if (line_end) begin
            last_iso <= iso_grant;
            owner    <= OWN_NONE;
        end
    end

    // =================================================================
    // The board's own display
    // =================================================================
    //
    // Each half's own `led[14:8]` is its status; its `led[7:0]` is a byte
    // this board has no room for twice, so those are dropped and the
    // fifteenth lamp says who has the port. `iso7816_terminal.v` and
    // `ssd1306_console.v` list what their seven bits mean, and the header
    // above repeats the result.
    assign led = {iso_led[14:8], disp_led[14:8], disp_grant};

    // The digits alternate between the two halves' own four-digit counts.
    // Each half drives a valid active-low anode pattern with the segments
    // that go with it, so selecting between the two pairs is sound however
    // their internal mux counters happen to line up.
    reg [SHOW_BIT:0] spin = {(SHOW_BIT + 1){1'b0}};
    always @(posedge clk) spin <= spin + 1'b1;
    wire show_disp = spin[SHOW_BIT];

    assign seg = show_disp ? disp_seg : iso_seg;
    assign an  = show_disp ? disp_an  : iso_an;
    // Active low, so this lights the point while the display half's frame
    // count is the thing on the glass. Without it the two counts would be
    // indistinguishable.
    assign dp  = ~show_disp;
endmodule
