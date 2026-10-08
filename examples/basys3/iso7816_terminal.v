// A working ISO 7816 terminal on a Digilent Basys 3: activate a card, read
// its answer, negotiate a faster rate, then talk to it — all from a host
// over the board's serial port.
//
// `iso7816_probe.v` beside this one only listens, which was enough to prove
// the wiring, the clock, the supply switch, the reset timing and the
// character framing against a real device. This one transmits as well, so
// it can do the two things that needed a transmitter: the PPS exchange that
// raises the rate, and sending command bytes afterwards.
//
// ===================================================================
// WIRING
// ===================================================================
//
//   JXADC1  J3   io         the card's single line, open drain, with an
//                           external 1k pull-up to 3.3 V
//   JXADC2  L3   clk_card   112 MHz / CARD_DIV; /14 is exactly 8 MHz
//   JXADC3  M2   rst_card   active low
//   JXADC4  N2   vcc_en     an AQV210 PhotoMOS relay's LED, through a
//                           series resistor
//   JXADC5  —    GND
//
// ===================================================================
// THE CONVERSATION
// ===================================================================
//
//   A          activate: power, clock, hold reset 400 card clocks, release
//   P          send PPS `FF 10 87 68`. The card's echo comes back as hex
//              like anything else; **the host compares it**, and sends `F`
//              if it agrees.
//   F          switch to the fast rate. `S` switches back.
// A byte printed with `>` in front came from the monitor, a second receiver
// on this contact that never drives it -- so it is what this terminal itself
// put on the wire. A byte with no prefix came from the card. `!` means the
// parity was wrong, whichever receiver saw it.
//
//   L          drive the line low for 18 ms, read it, release, read again:
//              `+PAD` if it went low and came back high, `-PAD` otherwise.
//              This is the only check that the pad drives at all, as
//              opposed to this design believing it transmitted.
//   :<hex>     send those bytes to the card. `:00A4040C` sends four. Hex
//              until anything that is not a hex digit ends it, so a newline
//              is a fine terminator.
//   D          deactivate: reset, clock, line, then power — in that order
//   s          status
//
// `A`, `D` and `P` are themselves hex digits, which is why sending bytes
// needs the `:` prefix: inside a `:` run every character is data, outside it
// every character is a command. Without that, `:0A` and a request to
// activate would be indistinguishable.
//
// Everything the card says comes back as two hex characters per byte, with
// `!` before a byte whose parity was wrong, and a CRLF once the line has
// been idle for `GAP_ETU` etu. Banners mark what the terminal did:
//
//   +VCC +CLK +RST   the activation sequence, step by step
//   +FST             the rate is now the fast one, from `F`
//   +SLW             back to the default, from `S`
//   -OFF             deactivated
//
// ===================================================================
// THE RATE, AND WHY BOTH VALUES ARE EXACT
// ===================================================================
//
// An etu is `F/D` card clock cycles, so in system cycles it is
// `(F/D) * CARD_DIV` — an integer at every rate this board offers, because
// 112 MHz was chosen so that every card frequency is an even integer
// division of it:
//
//   /14 = 8 MHz   /16 = 7   /20 = 5.6   /28 = 4
//   /32 = 3.5     /56 = 2   /64 = 1.75  /112 = 1
//
// Before PPS, `F/D` is 372: etu = 372 * 14 = **5208** system cycles, which
// is 21505 baud. After this card's PPS, `F/D` is 4: etu = 4 * 14 = **56**,
// which is 2 Mbaud. Changing `CARD_DIV` changes both rates together and
// keeps both exact, which is the whole reason the etu is expressed in card
// cycles rather than in a baud number.
//
// **`TA1 = 87` is partly vendor-specific.** `DI = 7` is Di = 64 as the
// standard says, but **`FI = 8` is RFU** in ISO 7816-3's Fi table — `Fi =
// 512` is FI = 9. So `F/D = 4` here comes from the device's own
// documentation and the owner's measurement, not from the standard's
// tables, and `FAST_ETU_CYCLES` is a parameter for exactly that reason.
//
// ===================================================================
// WHAT IS CHECKED
// ===================================================================
//
// The activation sequence, the 8E2 framing and the receive path are CHECKED
// against the owner's own device — `docs/fpga-xray.md` has the measurement,
// including the fourteen-byte answer-to-reset it produced. **The transmit
// path and the PPS exchange have not run against a card yet**; they are
// verified in simulation against a model that echoes a correct PPS and,
// separately, one that does not.

module iso7816_terminal #(
    // 112 MHz / CARD_DIV. 14 is 8 MHz, even so the duty is exactly 50 %.
    parameter CARD_DIV         = 14,
    // F/D before PPS. 372 is the standard default.
    parameter ETU_CYCLES       = 372,
    // F/D after this card's PPS. Vendor-specific; see above.
    parameter FAST_ETU_CYCLES  = 4,
    parameter RST_HOLD         = 400,
    parameter VCC_BITS         = 20,
    parameter GAP_ETU          = 256,
    parameter HOST_DIV         = 972,
    parameter WDOG_BITS        = 31
) (
    // The system clock, 112 MHz on the board. The top makes it, because a
    // PLL request is only honoured on the top module's nets.
    input  wire        clk,
    // High once that clock is trustworthy: the top wires the PLL's own
    // `LOCKED` here, and a testbench ties it high.
    input  wire        locked,

    output wire        clk_card,
    output wire        rst_card,
    output wire        vcc_en,

    // **The pad's three wires, not an `inout`.** Tristate belongs at the
    // top level: `examples/basys3/iso7816_terminal_pad.v` is the thin
    // wrapper that turns these into one bidirectional ball.
    //
    // Keeping it out of here is what makes the block testable. A testbench
    // that wants two devices on one open-drain wire cannot use two tristate
    // drivers — `reticle sim` refuses a net driven from more than one place,
    // and rightly, since without a pull-up two released drivers are X. With
    // `io_oe`/`io_o` exposed, a testbench models the bus as the wired-AND
    // that a pull-up physically is, and two devices can share it.
    input  wire        io_i,
    output wire        io_oe,
    output wire        io_o,

    input  wire        uart_rx_pin,
    output wire        uart_tx_pin,

    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    localparam integer SLOW_DIV = ETU_CYCLES * CARD_DIV;
    localparam integer FAST_DIV = FAST_ETU_CYCLES * CARD_DIV;
    localparam [15:0]  SLOW_DIV16 = SLOW_DIV;
    localparam [15:0]  FAST_DIV16 = FAST_DIV;
    localparam [7:0]   HALF_LAST  = (CARD_DIV / 2) - 1;
    localparam [15:0]  CARD_DIV16 = CARD_DIV;

    // **No PLL here.** `clock_mhz` and `clock_locked` are requests to the
    // backend and are only honoured on the **top** module's nets — wrapping
    // this core in `iso7816_terminal_pad` moved them into a submodule and
    // the build failed with `core.clk_pll` driven by nothing. So the top
    // owns the clocking as well as the pads, and this module is pure logic
    // with a clock input, which is the better split anyway: it is what lets
    // a testbench drive it from an ordinary clock with no PLL in sight.
    wire sys = clk;

    reg [15:0] por = 16'h0000;
    always @(posedge sys) por <= {por[14:0], locked};
    wire rst_n = por[15];

    // =================================================================
    // `L`: can this pad actually pull the line low?
    // =================================================================
    //
    // **This exists because `sent_q` is not evidence.** The terminal
    // counted four PPS bytes transmitted with no errors and the card
    // answered nothing -- but that counter reflects this design's own state
    // machine, not the wire. Receiving is proven on silicon by the ATR;
    // driving never has been, and it is the one link in the chain with no
    // measurement behind it.
    //
    // Against the line's 1 k pull-up the test is unambiguous: drive low and
    // the line must read low, release and it must read high. A pad that
    // cannot do that explains the silence completely, and one that can
    // moves the question to the card.
    localparam [23:0] PAD_HOLD = 24'd2_000_000;   // ~18 ms at 112 MHz

    reg        pad_drive = 1'b0;
    reg [1:0]  pad_step  = 2'd0;
    reg [23:0] pad_t     = 24'd0;
    reg        pad_lo    = 1'b1;   // the level read while driving low
    reg        pad_hi    = 1'b0;   // and after releasing

    wire blk_oe, blk_o;
    assign io_oe = blk_oe | pad_drive;
    assign io_o  = pad_drive ? 1'b0 : blk_o;

    always @(posedge sys) begin
        if (!rst_n) begin
            pad_drive <= 1'b0;
            pad_step  <= 2'd0;
            pad_t     <= 24'd0;
        end else case (pad_step)
            2'd0: if (cmd_valid && cmd_data == 8'h4C) begin   // 'L'
                pad_drive <= 1'b1;
                pad_t     <= PAD_HOLD;
                pad_step  <= 2'd1;
            end
            2'd1: begin
                pad_t <= pad_t - 1'b1;
                if (pad_t == 24'd1) begin
                    pad_lo    <= io_i;      // sampled while still driving
                    pad_drive <= 1'b0;
                    pad_t     <= PAD_HOLD;
                    pad_step  <= 2'd2;
                end
            end
            2'd2: begin
                pad_t <= pad_t - 1'b1;
                if (pad_t == 24'd1) begin
                    pad_hi   <= io_i;       // and after letting it rise
                    pad_step <= 2'd3;
                end
            end
            2'd3: pad_step <= 2'd0;         // the banner is raised below
        endcase
    end

    // `+PAD` only when it went low under drive and high when released.
    wire pad_done = (pad_step == 2'd3);
    wire pad_good = (pad_lo == 1'b0) && (pad_hi == 1'b1);

    // =================================================================
    // A monitor on the same contact
    // =================================================================
    //
    // **Reusing the receiver a real card already validated, instead of
    // instrumentation written this afternoon.** A second `iso7816_uart`
    // that never transmits -- `tx_valid` tied low, its `io_oe` and `io_o`
    // left unconnected, so it cannot touch the wire. It decodes everything
    // that appears there, including this terminal's own transmission, which
    // the main instance suppresses from its own receiver.
    //
    // It answers the question a trace buffer was being built for: are the
    // four PPS characters well formed on the wire? And it answers with a
    // block that has conformance evidence -- it read a real card's 14-byte
    // ATR with no parity or framing error -- rather than with new logic
    // that had none. The trace buffer it replaces could not be checked in
    // simulation, because its sample interval was written out instead of
    // derived; it then failed on the board for a third reason, an edit that
    // silently never applied, leaving the thing that requested the dump
    // declared, read, and never assigned. "The instrument is broken" and
    // "the line never moved" look identical from the far end of a serial
    // port, and telling them apart cost two board runs.
    //
    // Its bytes print with `>` in front and the card's with nothing, so a
    // single line says who drove the contact.
    wire [7:0] mon_rx;
    wire       mon_rx_valid, mon_parity_err;

    iso7816_uart #(.ETU_DIV(SLOW_DIV)) monitor (
        .clk(sys), .rst_n(rst_n),
        .active(card_active),
        .etu_div(etu_div),
        .guard_etu(8'd0),
        .convention(1'b0),
        .wt_etu(24'd0),                   // it watches; it never waits
        .tx_data(8'd0), .tx_valid(1'b0), .tx_ready(),
        .tx_abort(),
        .rx_data(mon_rx), .rx_ready(1'b1), .rx_valid(mon_rx_valid),
        .rx_parity_error(mon_parity_err), .rx_overrun(),
        .rx_timeout(),
        .io_i(io_i), .io_oe(), .io_o(),
        .tx_char_count(), .rx_char_count(), .parity_error_count(),
        .repeat_count(), .timeout_count());

    // =================================================================
    // The card clock
    // =================================================================

    reg [7:0] cdiv   = 8'd0;
    reg       cclk   = 1'b0;
    reg       crise  = 1'b0;
    reg       clk_on = 1'b0;

    always @(posedge sys) begin
        crise <= 1'b0;
        if (!rst_n || !clk_on) begin
            cdiv <= 8'd0;
            cclk <= 1'b0;
        end else if (cdiv == HALF_LAST) begin
            cdiv  <= 8'd0;
            cclk  <= ~cclk;
            crise <= ~cclk;
        end else begin
            cdiv <= cdiv + 8'd1;
        end
    end

    assign clk_card = cclk;

    // =================================================================
    // The host's serial port
    // =================================================================

    wire [7:0] cmd_data;
    wire       cmd_valid;
    wire       host_ready;
    reg  [7:0] host_data  = 8'd0;
    reg        host_valid = 1'b0;

    uart #(.CLK_DIV(HOST_DIV)) host (
        .clk(sys), .rst_n(rst_n), .div(16'd0),
        .tx_data(host_data), .tx_valid(host_valid), .tx_ready(host_ready),
        .tx(uart_tx_pin),
        .rx(uart_rx_pin), .rx_data(cmd_data), .rx_valid(cmd_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

    function is_hex_char;
        input [7:0] c;
        is_hex_char = (c >= 8'd48 && c <= 8'd57)      // 0-9
                   || (c >= 8'd65 && c <= 8'd70)      // A-F
                   || (c >= 8'd97 && c <= 8'd102);    // a-f
    endfunction

    // Through an eight-bit local and then a slice: `c - 8'd48` is eight
    // bits wide and assigning it straight to a four-bit result truncates,
    // which the simulator rightly warns about even though the value always
    // fits.
    function [3:0] hex_val;
        input [7:0] c;
        reg [7:0] sub;
        begin
            sub = (c <= 8'd57) ? (c - 8'd48)
                : (c <= 8'd70) ? (c - 8'd55)
                :                (c - 8'd87);
            hex_val = sub[3:0];
        end
    endfunction

    // =================================================================
    // The card's line
    // =================================================================

    reg  [15:0] etu_div = SLOW_DIV16;
    reg         card_active = 1'b0;

    wire [7:0]  card_rx;
    wire        card_rx_valid, card_parity_err, card_timeout;
    wire        card_tx_ready;
    reg  [7:0]  card_tx = 8'd0;
    reg         card_tx_valid = 1'b0;
    iso7816_uart #(.ETU_DIV(SLOW_DIV)) line (
        .clk(sys), .rst_n(rst_n),
        .active(card_active),
        .etu_div(etu_div),
        .guard_etu(8'd0),
        .convention(1'b0),            // direct: TS = 3B
        .wt_etu(24'd9600),            // T=0 work waiting time
        .tx_data(card_tx), .tx_valid(card_tx_valid), .tx_ready(card_tx_ready),
        .tx_abort(),
        .rx_data(card_rx), .rx_ready(1'b1), .rx_valid(card_rx_valid),
        .rx_parity_error(card_parity_err), .rx_overrun(),
        .rx_timeout(card_timeout),
        .io_i(io_i), .io_oe(blk_oe), .io_o(blk_o),
        .tx_char_count(), .rx_char_count(), .parity_error_count(),
        .repeat_count(), .timeout_count());

    // =================================================================
    // Sending bytes: a `:` run from the host, or the PPS
    // =================================================================

    localparam [1:0] SRC_IDLE = 2'd0, SRC_HOST = 2'd1, SRC_PPS = 2'd2;

    reg [1:0] tx_src   = SRC_IDLE;
    reg       in_hex   = 1'b0;      // inside a `:` run
    reg       have_hi  = 1'b0;
    reg [3:0] hi_nib   = 4'd0;
    reg [1:0]  pps_idx = 2'd0;
    // Four etu of silence between the characters of a sequence this module
    // sends itself, which is comfortably more than the two the standard
    // requires and costs nothing at either rate.
    reg [19:0] tx_wait = 20'd0;
    wire [19:0] gap_etu_4 = {etu_div, 2'd0};

    // `FF 10 87 68`: PPSS, then PPS0 saying PPS1 follows and the protocol
    // is T=0, then PPS1 carrying the same Fi/Di as TA1, then the check
    // byte, which is the exclusive-or of the three before it.
    function [7:0] pps_byte;
        input [1:0] at;
        case (at)
            2'd0: pps_byte = 8'hFF;
            2'd1: pps_byte = 8'h10;
            2'd2: pps_byte = 8'h87;
            default: pps_byte = 8'h68;
        endcase
    endfunction

    // What the card must echo, and how much of it has matched so far.
    reg       pps_armed = 1'b0;

    always @(posedge sys) begin
        if (!rst_n) begin
            in_hex  <= 1'b0;
            have_hi <= 1'b0;
        end else if (cmd_valid) begin
            if (cmd_data == 8'h3A) begin        // ':'
                in_hex  <= 1'b1;
                have_hi <= 1'b0;
            end else if (in_hex && !is_hex_char(cmd_data)) begin
                in_hex  <= 1'b0;
                have_hi <= 1'b0;
            end else if (in_hex && is_hex_char(cmd_data)) begin
                if (!have_hi) begin
                    hi_nib  <= hex_val(cmd_data);
                    have_hi <= 1'b1;
                end else begin
                    have_hi <= 1'b0;
                end
            end
        end
    end

    // A byte is complete when the second nibble of a pair arrives.
    wire host_byte_ready = cmd_valid && in_hex && is_hex_char(cmd_data)
                        && have_hi;
    wire [7:0] host_byte = {hi_nib, hex_val(cmd_data)};

    // The guard-time countdown has to run while `card_tx_valid` is **low**,
    // which is why it is inside that branch and not after it. The first
    // version put it in a later `else if`, reachable only when valid was
    // high — so after the first byte the timer never decremented and the
    // sequence stalled. The testbench reported only "did not finish",
    // which is why `card_send` there is now bounded and says why it gave up.
    always @(posedge sys) begin
        if (!rst_n) begin
            card_tx_valid <= 1'b0;
            tx_src        <= SRC_IDLE;
            pps_idx       <= 2'd0;
            tx_wait       <= 20'd0;
        end else if (!card_tx_valid) begin
            if (tx_src == SRC_PPS && tx_wait != 0) begin
                tx_wait <= tx_wait - 1'b1;
                if (tx_wait == 20'd1) begin
                    card_tx       <= pps_byte(pps_idx);
                    card_tx_valid <= 1'b1;
                    pps_idx       <= (pps_idx == 2'd3) ? 2'd0 : (pps_idx + 2'd1);
                end
            end else if (host_byte_ready) begin
                card_tx       <= host_byte;
                card_tx_valid <= 1'b1;
                tx_src        <= SRC_HOST;
            end else if (pps_armed && tx_src != SRC_PPS) begin
                card_tx       <= pps_byte(2'd0);
                card_tx_valid <= 1'b1;
                tx_src        <= SRC_PPS;
                pps_idx       <= 2'd1;
            end else if (tx_src == SRC_PPS) begin
                // The fourth byte has gone and no wait is pending.
                tx_src <= SRC_IDLE;
            end
        end else if (card_tx_ready) begin
            card_tx_valid <= 1'b0;
            // Four etu of silence before the next character of our own
            // sequence. Presenting it the instant `tx_ready` rises assumes
            // the block enforces the guard, and `pair_tb` shows bytes
            // separated by several etu exchange cleanly while back-to-back
            // ones draw a parity error and a T=0 repeat.
            if (tx_src == SRC_PPS && pps_idx != 2'd0) tx_wait <= gap_etu_4;
        end
    end

    // **The host judges the echo, not the hardware.**
    //
    // This module used to match the card's PPS response itself and switch
    // the rate when all four bytes agreed. That cost four separate bugs and
    // a day: the echo arrives at the old rate, the switch must happen after
    // the last byte and not before, a mis-received byte makes the terminal
    // signal a parity error that makes the card repeat, and repeats that
    // exhaust leave the card unable to transmit at all. Every one of those
    // is state this module does not need to hold.
    //
    // Instead `P` transmits the four bytes and the echo comes back as hex
    // like anything else, and `F` switches to the fast rate once the host
    // has read it. A host comparing four bytes is three lines of script; the
    // same comparison here was a state machine that could desynchronise
    // from the card in a way neither side could recover from. This is the
    // rule every other design here follows — complexity belongs where it is
    // cheap — and abandoning it for this one was the mistake.
    //
    // Nothing is lost in safety: the rate cannot change by accident, only
    // on an explicit command, and the host that sends it has just seen the
    // echo it is deciding on.
    reg [2:0] banner = 3'd0;  // 1 +VCC 2 +CLK 3 +RST 4 -OFF 5 +FAST 6 +SLOW

    // =================================================================
    // The sequencer
    // =================================================================

    localparam [2:0] S_IDLE = 3'd0, S_VCC = 3'd1, S_CLK = 3'd2,
                     S_RUN  = 3'd3, S_DOWN = 3'd4, S_OFF = 3'd5;

    reg [2:0]        state  = S_IDLE;
    reg [VCC_BITS:0] settle = {(VCC_BITS + 1){1'b0}};
    reg [15:0]       held   = 16'd0;
    reg              vcc_q  = 1'b0;
    reg              rst_q  = 1'b0;

    reg [WDOG_BITS:0] wdog = {(WDOG_BITS + 1){1'b0}};
    wire wdog_bite = (state == S_RUN || state == S_VCC || state == S_CLK)
                   && (wdog == 0);

    always @(posedge sys) begin
        if (!rst_n) wdog <= {(WDOG_BITS + 1){1'b0}};
        else if (cmd_valid || card_rx_valid) wdog <= {1'b1, {WDOG_BITS{1'b0}}};
        else if (wdog != 0) wdog <= wdog - 1'b1;
    end

    always @(posedge sys) begin
        if (!rst_n) begin
            state       <= S_IDLE;
            vcc_q       <= 1'b0;
            rst_q       <= 1'b0;
            clk_on      <= 1'b0;
            card_active <= 1'b0;
            settle      <= {(VCC_BITS + 1){1'b0}};
            held        <= 16'd0;
            banner      <= 3'd0;
            etu_div     <= SLOW_DIV16;
            pps_armed   <= 1'b0;
        end else begin
            banner <= 3'd0;      // a pulse, not a level
            case (state)
                S_IDLE:
                    if (cmd_valid && cmd_data == 8'h41) begin   // 'A'
                        vcc_q   <= 1'b1;
                        settle  <= {1'b1, {VCC_BITS{1'b0}}};
                        etu_div <= SLOW_DIV16;
                        banner  <= 3'd1;
                        state   <= S_VCC;
                    end
                S_VCC:
                    if (wdog_bite) begin
                        vcc_q  <= 1'b0; clk_on <= 1'b0;
                        banner <= 3'd4; state <= S_OFF;
                    end else if (settle != 0) settle <= settle - 1'b1;
                    else begin
                        clk_on <= 1'b1;
                        held   <= 16'd0;
                        banner <= 3'd2;
                        state  <= S_CLK;
                    end
                S_CLK:
                    if (wdog_bite) begin
                        rst_q <= 1'b0; clk_on <= 1'b0; vcc_q <= 1'b0;
                        banner <= 3'd4; state <= S_OFF;
                    end else if (crise) begin
                        if (held == RST_HOLD - 1) begin
                            rst_q       <= 1'b1;
                            card_active <= 1'b1;
                            banner      <= 3'd3;
                            state       <= S_RUN;
                        end else begin
                            held <= held + 16'd1;
                        end
                    end
                S_RUN: begin
                    if (cmd_valid && cmd_data == 8'h50) pps_armed <= 1'b1;
                    if (pps_armed && tx_src == SRC_PPS && pps_idx == 2'd0
                        && !card_tx_valid)
                        pps_armed <= 1'b0;     // all four have gone
                    // The host decides, having seen the echo.
                    if (cmd_valid && cmd_data == 8'h46) begin      // 'F'
                        etu_div <= FAST_DIV16;
                        banner  <= 3'd5;
                    end
                    if (cmd_valid && cmd_data == 8'h53) begin      // 'S'
                        etu_div <= SLOW_DIV16;
                        banner  <= 3'd6;
                    end
                    if (pad_done) banner <= 3'd7;
                    if ((cmd_valid && cmd_data == 8'h44) || wdog_bite) begin
                        rst_q       <= 1'b0;
                        clk_on      <= 1'b0;
                        card_active <= 1'b0;
                        settle      <= {1'b1, {VCC_BITS{1'b0}}};
                        state       <= S_DOWN;
                    end
                end
                S_DOWN:
                    if (settle != 0) settle <= settle - 1'b1;
                    else begin
                        vcc_q  <= 1'b0;
                        banner <= 3'd4;
                        state  <= S_OFF;
                    end
                default: state <= S_IDLE;
            endcase
        end
    end

    assign vcc_en   = vcc_q;
    assign rst_card = rst_q;

    // =================================================================
    // Counters
    // =================================================================

    reg [15:0] bytes_q = 16'd0, perr_q = 16'd0, sent_q = 16'd0,
               tmo_q   = 16'd0;
    always @(posedge sys) begin
        if (!rst_n) begin
            bytes_q <= 16'd0; perr_q <= 16'd0; sent_q <= 16'd0;
            tmo_q   <= 16'd0;
        end else begin
            if (card_rx_valid) begin
                if (bytes_q != 16'hFFFF) bytes_q <= bytes_q + 16'd1;
                if (card_parity_err && perr_q != 16'hFFFF)
                    perr_q <= perr_q + 16'd1;
            end
            if (card_tx_valid && card_tx_ready && sent_q != 16'hFFFF)
                sent_q <= sent_q + 16'd1;
            if (card_timeout && tmo_q != 16'hFFFF) tmo_q <= tmo_q + 16'd1;
        end
    end

    // =================================================================
    // Printing
    // =================================================================

    localparam [2:0] P_IDLE = 3'd0, P_BANNER = 3'd1, P_BYTE = 3'd2,
                     P_EOL  = 3'd3, P_STATUS = 3'd4;

    reg [2:0]   phase = P_IDLE;
    reg [3:0]   pos   = 4'd0;
    reg [7:0]   pend_byte = 8'd0;
    reg         pend_bad  = 1'b0;
    reg         pend_mon  = 1'b0;
    reg         have_byte = 1'b0;
    reg [127:0] status_sh = 128'd0;
    reg [5:0]   status_left = 6'd0;
    reg         want_status = 1'b0;
    reg         want_known  = 1'b0;
    reg [31:0]  gap = 32'd0;
    reg         line_open = 1'b0;

    localparam [31:0] GAP_LOAD_SLOW = SLOW_DIV * GAP_ETU;
    // The gap is in etu, so it must shrink with the rate too, or a fast
    // exchange would wait a slow line's worth of time before ending a line.
    wire [31:0] gap_load = (etu_div == FAST_DIV16)
                         ? (FAST_DIV * GAP_ETU)
                         : GAP_LOAD_SLOW;

    reg p_vcc = 1'b0, p_clk = 1'b0, p_rst = 1'b0, p_off = 1'b0,
        p_fast = 1'b0, p_slow = 1'b0, p_pad = 1'b0;
    wire [2:0] want_banner = p_vcc ? 3'd1 : p_clk ? 3'd2 : p_rst ? 3'd3
                           : p_fast ? 3'd5 : p_slow ? 3'd6 : p_pad ? 3'd7 : p_off ? 3'd4
                           : 3'd0;

    wire banner_done = (phase == P_BANNER) && (pos == 4'd5) && host_valid && host_ready;
    wire byte_done   = (phase == P_BYTE)   && (pos == 4'd2) && host_valid && host_ready;
    wire eol_done    = (phase == P_EOL)    && (pos == 4'd1) && host_valid && host_ready;

    always @(posedge sys) begin
        if (!rst_n) begin
            p_vcc <= 1'b0; p_clk <= 1'b0; p_rst <= 1'b0; p_off <= 1'b0;
            p_fast <= 1'b0; p_slow <= 1'b0; p_pad <= 1'b0;
        end else begin
            case (banner)
                3'd1: p_vcc <= 1'b1;
                3'd2: p_clk <= 1'b1;
                3'd3: p_rst <= 1'b1;
                3'd4: p_off <= 1'b1;
                3'd5: p_fast <= 1'b1;
                3'd6: p_slow <= 1'b1;
                3'd7: p_pad  <= 1'b1;
                default: ;
            endcase
            if (banner_done) begin
                case (want_banner)
                    3'd1: p_vcc <= 1'b0;
                    3'd2: p_clk <= 1'b0;
                    3'd3: p_rst <= 1'b0;
                    3'd5: p_fast <= 1'b0;
                    3'd6: p_slow <= 1'b0;
                    3'd7: p_pad  <= 1'b0;
                    default: p_off <= 1'b0;
                endcase
            end
        end
    end

    always @(posedge sys) begin
        if (!rst_n) begin
            have_byte <= 1'b0;
            gap       <= 32'd0;
            line_open <= 1'b0;
        end else begin
            if (card_rx_valid) begin
                pend_byte <= card_rx;
                pend_bad  <= card_parity_err;
                pend_mon  <= 1'b0;
                have_byte <= 1'b1;
                gap       <= gap_load;
                line_open <= 1'b1;
            end else if (mon_rx_valid) begin
                // The card's bytes arrive on both receivers and the main one
                // wins the cycle, so what reaches here is what this terminal
                // put on the wire itself.
                pend_byte <= mon_rx;
                pend_bad  <= mon_parity_err;
                pend_mon  <= 1'b1;
                have_byte <= 1'b1;
                gap       <= gap_load;
                line_open <= 1'b1;
            end else if (gap != 0) begin
                gap <= gap - 1'b1;
            end
            if (byte_done) have_byte <= 1'b0;
            if (eol_done)  line_open <= 1'b0;
        end
    end

    always @(posedge sys) begin
        if (!rst_n) want_status <= 1'b0;
        else if (cmd_valid && cmd_data == 8'h73) want_status <= 1'b1;
        else if (phase == P_STATUS && status_left == 0 && pos == 4'd1
                 && host_valid && host_ready) want_status <= 1'b0;
    end

    // `k`: the same emitter, a known constant, so that a reading can be
    // separated from the thing it reads.
    always @(posedge sys) begin
        if (!rst_n) want_known <= 1'b0;
        else if (cmd_valid && cmd_data == 8'h6B) want_known <= 1'b1;
        else if (phase == P_STATUS && status_left == 0 && pos == 4'd1
                 && host_valid && host_ready) want_known <= 1'b0;
    end

    function [7:0] hex;
        input [3:0] n;
        hex = (n < 4'd10) ? (8'd48 + {4'd0, n}) : (8'd55 + {4'd0, n});
    endfunction

    function [7:0] banner_ch;
        input [2:0] which;
        input [1:0] at;
        case ({which, at})
            {3'd1, 2'd0}: banner_ch = "+";
            {3'd1, 2'd1}: banner_ch = "V";
            {3'd1, 2'd2}: banner_ch = "C";
            {3'd1, 2'd3}: banner_ch = "C";
            {3'd2, 2'd0}: banner_ch = "+";
            {3'd2, 2'd1}: banner_ch = "C";
            {3'd2, 2'd2}: banner_ch = "L";
            {3'd2, 2'd3}: banner_ch = "K";
            {3'd3, 2'd0}: banner_ch = "+";
            {3'd3, 2'd1}: banner_ch = "R";
            {3'd3, 2'd2}: banner_ch = "S";
            {3'd3, 2'd3}: banner_ch = "T";
            {3'd4, 2'd0}: banner_ch = "-";
            {3'd4, 2'd1}: banner_ch = "O";
            {3'd4, 2'd2}: banner_ch = "F";
            {3'd4, 2'd3}: banner_ch = "F";
            {3'd5, 2'd0}: banner_ch = "+";
            {3'd5, 2'd1}: banner_ch = "F";
            {3'd5, 2'd2}: banner_ch = "S";
            {3'd7, 2'd0}: banner_ch = pad_good ? "+" : "-";
            {3'd7, 2'd1}: banner_ch = "P";
            {3'd7, 2'd2}: banner_ch = "A";
            {3'd7, 2'd3}: banner_ch = "D";
            {3'd6, 2'd0}: banner_ch = "+";
            {3'd6, 2'd1}: banner_ch = "S";
            {3'd6, 2'd2}: banner_ch = "L";
            default:      banner_ch = "T";
        endcase
    endfunction

    wire [127:0] status = {
        bytes_q, perr_q, sent_q, tmo_q,
        etu_div, CARD_DIV16,
        {7'd0, etu_div == FAST_DIV16}, {5'd0, state},
        8'hA5,
        // The two samples `L` took, because `-PAD` alone does not say which
        // half failed and they mean different things: a line that would not
        // go low is an output that never reached the pin, while one that
        // stayed low is an enable that never released.
        {5'd0, pad_lo, pad_hi, pad_done}
    };

    always @(posedge sys) begin
        if (!rst_n) begin
            phase      <= P_IDLE;
            pos        <= 4'd0;
            host_valid <= 1'b0;
        end else if (!host_valid) begin
            case (phase)
                P_IDLE:
                    if (want_banner != 3'd0)      begin phase <= P_BANNER; pos <= 4'd0; end
                    else if (want_known) begin
                        // Every nibble distinct. If this comes back
                        // altered, the fault is the path or the way
                        // this flow builds a constant, not the
                        // counters.
                        status_sh   <= 128'h0123456789ABCDEFFEDCBA9876543210;
                        status_left <= 6'd32;
                        phase       <= P_STATUS;
                        pos         <= 4'd0;
                    end else if (want_status) begin
                        status_sh   <= status;
                        status_left <= 6'd32;
                        phase       <= P_STATUS;
                        pos         <= 4'd0;
                    end else if (have_byte) begin
                        phase <= P_BYTE;
                        pos   <= (pend_bad || pend_mon) ? 4'd0 : 4'd1;
                    end else if (line_open && gap == 0) begin
                        phase <= P_EOL; pos <= 4'd0;
                    end
                P_BANNER: begin
                    host_data  <= (pos < 4'd4) ? banner_ch(want_banner, pos[1:0])
                                : (pos == 4'd4) ? 8'd13 : 8'd10;
                    host_valid <= 1'b1;
                end
                P_BYTE: begin
                    host_data  <= (pos == 4'd0) ? (pend_mon ? 8'h3E : 8'h21)
                                : (pos == 4'd1) ? hex(pend_byte[7:4])
                                :                 hex(pend_byte[3:0]);
                    host_valid <= 1'b1;
                end
                P_EOL: begin
                    host_data  <= (pos == 4'd0) ? 8'd13 : 8'd10;
                    host_valid <= 1'b1;
                end
                default: begin
                    host_data  <= (status_left != 0) ? hex(status_sh[127:124])
                                : (pos == 4'd0) ? 8'd13 : 8'd10;
                    host_valid <= 1'b1;
                end
            endcase
        end else if (host_ready) begin
            host_valid <= 1'b0;
            case (phase)
                P_BANNER: if (pos == 4'd5) begin phase <= P_IDLE; pos <= 4'd0; end
                          else pos <= pos + 4'd1;
                P_BYTE:   if (pos == 4'd2) begin phase <= P_IDLE; pos <= 4'd0; end
                          else pos <= pos + 4'd1;
                P_EOL:    if (pos == 4'd1) begin phase <= P_IDLE; pos <= 4'd0; end
                          else pos <= pos + 4'd1;
                default:
                    if (status_left != 0) begin
                        status_sh   <= {status_sh[123:0], 4'd0};
                        status_left <= status_left - 6'd1;
                    end else if (pos == 4'd1) begin
                        phase <= P_IDLE; pos <= 4'd0;
                    end else begin
                        pos <= pos + 4'd1;
                    end
            endcase
        end
    end

    // =================================================================
    // The board's own display
    // =================================================================

    reg [25:0] tick = 26'd0;
    always @(posedge sys) tick <= tick + 26'd1;

    assign led = {tick[25], perr_q != 0, etu_div == FAST_DIV16, locked,
                  rst_q, clk_on, vcc_q, bytes_q[7:0]};

    reg [17:0] mux = 18'd0;
    always @(posedge sys) mux <= mux + 18'd1;
    wire [1:0] digit = mux[17:16];

    wire [3:0] nibble =
        (digit == 2'd0) ? bytes_q[3:0]   :
        (digit == 2'd1) ? bytes_q[7:4]   :
        (digit == 2'd2) ? bytes_q[11:8]  :
                          bytes_q[15:12];

    function [6:0] segments;
        input [3:0] value;
        case (value)
            4'h0: segments = 7'b1000000;
            4'h1: segments = 7'b1111001;
            4'h2: segments = 7'b0100100;
            4'h3: segments = 7'b0110000;
            4'h4: segments = 7'b0011001;
            4'h5: segments = 7'b0010010;
            4'h6: segments = 7'b0000010;
            4'h7: segments = 7'b1111000;
            4'h8: segments = 7'b0000000;
            4'h9: segments = 7'b0010000;
            4'hA: segments = 7'b0001000;
            4'hB: segments = 7'b0000011;
            4'hC: segments = 7'b1000110;
            4'hD: segments = 7'b0100001;
            4'hE: segments = 7'b0000110;
            default: segments = 7'b0001110;
        endcase
    endfunction

    assign seg = segments(nibble);
    assign dp  = ~tick[25];
    assign an  = ~(4'd1 << digit);
endmodule
