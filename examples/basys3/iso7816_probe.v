// Bring an ISO 7816 card up and print whatever it answers.
//
// The first thing to run against a real device: switch its power on, give
// it a clock, hold reset the length the standard demands, release it, and
// forward every character it sends to a host over the board's own serial
// port. Nothing is transmitted on the card's line — this is receive only,
// which is all an answer-to-reset needs and the smallest thing that can
// prove the wiring, the clock, the power switch, the reset timing and the
// character framing all at once.
//
// ===================================================================
// WIRING
// ===================================================================
//
//   JXADC1  J3   io         the card's single bidirectional line. Read
//                           only here, with the external 1k pull-up
//                           holding it high between characters.
//   JXADC2  L3   clk_card   the clock the card derives everything from
//   JXADC3  M2   rst_card   active low
//   JXADC4  N2   vcc_en     a PhotoMOS relay's LED, through a series
//                           resistor: high turns the card's supply on
//   JXADC5  —    GND
//
// ===================================================================
// THE SEQUENCE, AND WHY IN THAT ORDER
// ===================================================================
//
// Activation, on `A`:
//
//   1. `vcc_en` high, then wait `VCC_SETTLE` system clocks. The relay is
//      optically coupled and takes a millisecond or two; the default here
//      is far longer than that, and it costs nothing because reset is
//      still held.
//   2. start `clk_card`.
//   3. wait `RST_HOLD` **card clock cycles** — not a fixed time. ISO
//      7816-3 states the hold as 400 clock cycles and sets no upper
//      bound, so counting the card's own clock makes the hold correct at
//      every frequency with no recalculation.
//   4. release `rst_card`.
//
// **Power before clock** is deliberate and not interchangeable. Driving a
// clock into an unpowered device forward-biases its input protection
// diodes, which can partly power it, confuse its reset, or damage it.
// Deactivation on `D` is the exact reverse: reset low, clock stopped low,
// line released, settle, then power off.
//
// ===================================================================
// WHAT IT PRINTS
// ===================================================================
//
// Two hexadecimal characters per byte the card sends, as they arrive, and
// a CRLF once the line has been idle for `GAP_ETU` etu — so an
// answer-to-reset arrives as one line. A byte whose parity was wrong is
// prefixed with `!`, and a framing error with `?`, because a corrupted
// answer is far more useful than a missing one when the convention or the
// rate is still in question.
//
// Two lines frame the attempt so a host never has to guess what it is
// looking at:
//
//   `+VCC`   power on, waiting for the relay
//   `+CLK`   clock running, holding reset
//   `+RST`   reset released; anything after this is the card talking
//   `-OFF`   deactivated
//
// `s` prints a status line: state, bytes received, parity errors, framing
// errors, and the etu divisor in use.
//
// ===================================================================
// THE CLOCK, AND WHY 112 MHz
// ===================================================================
//
// Everything runs on a PLL output of **112 MHz**, asked for by frequency
// and delivered at +0.0 ppm. That number is chosen so the card clock is an
// exact integer division with an even divisor, which keeps the duty cycle
// at exactly 50 %:
//
//   /14 = 8 MHz   /16 = 7      /20 = 5.6   /28 = 4
//   /32 = 3.5     /56 = 2      /64 = 1.75  /112 = 1
//
// and so that the etu follows automatically. An etu is `F/D` card clock
// cycles, so in system cycles it is `(F/D) * CARD_DIV` — an exact integer
// at every rate above. At the default `F/D` of 372 and `/14`, that is
// 5208 cycles, which is 21505 baud. After a PPS exchange `F/D` shrinks:
// `F/D = 4` gives 56 cycles, which is 2 Mbaud.
//
// `docs/fpga-xray.md` records what this costs: a PLL puts two tiles in the
// bitstream that have no segbits file, so those bits cannot be decoded
// back and checked. The part demonstrably locks and counts at the ratio it
// was asked for, so they are right — but right and verified are different
// claims and this one is only the first.
//
// ===================================================================
// WHAT THIS DOES NOT DO
// ===================================================================
//
// It never drives the card's line, so it cannot send a PPS, and the rate
// stays at the default. It assumes the **direct convention** (TS = 0x3B:
// logic one is high, least significant bit first); a card answering with
// the inverse convention (TS = 0x3F) will come out as recognisable
// nonsense rather than nothing, which is the point of printing raw hex
// and of marking parity errors instead of hiding them. The full character
// layer with both conventions, transmission and PPS is
// `ip/bus/iso7816_uart`.
//
// **Nothing here has run against a card.** When it has, this header says
// what it saw.

module iso7816_probe #(
    // Card clock = 112 MHz / CARD_DIV. 14 is 8 MHz. Even, for 50 % duty.
    parameter CARD_DIV    = 14,
    // F/D: card clock cycles per etu. 372 is the default before PPS.
    parameter ETU_CYCLES  = 372,
    // Reset held this many card clock cycles. ISO 7816-3 says at least 400.
    parameter RST_HOLD    = 400,
    // Power settle, in system clocks. 2^20 at 112 MHz is 9.4 ms, which is
    // several times an optically coupled relay's turn-on.
    parameter VCC_BITS    = 20,
    // Idle this many **etu** and the line is ended. In etu and not in
    // system clocks, because a character is 12 etu and the etu scales with
    // the card clock: a timeout counted in clocks ends the line between
    // bytes at a slow card frequency and behaves at a fast one — a fault
    // that appears only when the clock is swept, which is the plan.
    parameter GAP_ETU     = 24,
    // Host serial port: 112 MHz / 115200 = 972.
    parameter HOST_DIV    = 972,
    // The watchdog's period, as a power of two system clocks. 31 is 19 s
    // at 112 MHz.
    parameter WDOG_BITS   = 31,
    // 1 on the board: run everything on a 112 MHz PLL output. 0 in a
    // testbench: run on the input clock, because a PLL does not exist in a
    // netlist. See the note beside the declaration.
    parameter USE_PLL     = 1
) (
    input  wire        clk,

    output wire        clk_card,
    output wire        rst_card,
    output wire        vcc_en,
    input  wire        io,

    input  wire        uart_rx_pin,
    output wire        uart_tx_pin,

    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    localparam integer ETU_DIV = ETU_CYCLES * CARD_DIV;
    // Derived, so it tracks the card clock like everything else here.
    localparam [31:0]  GAP_LOAD = ETU_DIV * GAP_ETU;
    // Half a card clock period, less one, so the toggle gives an exactly
    // even split. Declared here because the card clock below uses it.
    localparam [7:0]   HALF_LAST = (CARD_DIV / 2) - 1;
    // Sized rather than cast: `reticle sim` rejects a SystemVerilog width
    // cast that `reticle check` accepts, so every constant that lands in a
    // sized context is a sized `localparam`.
    localparam [15:0]  ETU_DIV16   = ETU_DIV;
    localparam [15:0]  CARD_DIV16  = CARD_DIV;
    localparam [15:0]  RST_HOLD16  = RST_HOLD;

    // ---- 112 MHz, and nothing runs until it has locked. ----
    //
    // **`USE_PLL = 0` is what makes this design simulable at all**, and the
    // reason is worth stating because it is not obvious: `clock_mhz` and
    // `clock_locked` are *requests to the backend*, not logic. In a netlist
    // they are undriven wires — so a design clocked by a PLL output has, in
    // simulation, **no clock and no lock**, and never leaves reset. That is
    // why `examples/basys3/pll_blink.v` has no testbench, and why the first
    // attempt at this one hung with `vcc_en` never rising.
    //
    // With `USE_PLL = 0` the design runs on the input clock and lock is
    // assumed, which is what `iso7816_probe_tb.v` uses; the board build
    // leaves it at 1. Both fold to nothing at elaboration, so the choice
    // costs no logic either way.
    (* clock_mhz = 112 *) wire clk_pll;
    (* clock_locked = "clk_pll" *) wire pll_locked_raw;

    wire sys        = USE_PLL ? clk_pll        : clk;
    wire pll_locked = USE_PLL ? pll_locked_raw : 1'b1;

    // Saturating, and gated on lock: ones shift in so it cannot un-reset,
    // and it cannot start until the PLL says its output is real.
    reg [15:0] por = 16'h0000;
    always @(posedge sys) por <= {por[14:0], pll_locked};
    wire rst_n = por[15];

    // =================================================================
    // The card clock
    // =================================================================

    reg [7:0] cdiv  = 8'd0;
    reg       cclk  = 1'b0;
    reg       crise = 1'b0;      // one system clock per card clock rise
    reg       clk_on = 1'b0;

    always @(posedge sys) begin
        crise <= 1'b0;
        if (!rst_n) begin
            cdiv <= 8'd0;
            cclk <= 1'b0;
        end else if (!clk_on) begin
            // Stopped low, which is where a card expects it between
            // activations.
            cdiv <= 8'd0;
            cclk <= 1'b0;
        end else if (cdiv == HALF_LAST) begin
            cdiv  <= 8'd0;
            cclk  <= ~cclk;
            crise <= ~cclk;      // about to become high
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
    wire       tx_ready;
    reg  [7:0] tx_data  = 8'd0;
    reg        tx_valid = 1'b0;

    uart #(.CLK_DIV(HOST_DIV)) host (
        .clk(sys), .rst_n(rst_n), .div(16'd0),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
        .tx(uart_tx_pin),
        .rx(uart_rx_pin), .rx_data(cmd_data), .rx_valid(cmd_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

    // =================================================================
    // The card's line: 8E2 at the etu above
    // =================================================================
    //
    // Through two flip-flops first. The line is asynchronous to this clock
    // and an unsynchronised input into a receiver's edge detector is a
    // real race, not a theoretical one.
    reg [1:0] io_sync = 2'b11;
    always @(posedge sys) io_sync <= {io_sync[0], io};

    wire [7:0] card_byte;
    wire       card_valid, card_parity_err, card_frame_err;

    uart_frame_rx #(.CLK_DIV(ETU_DIV)) card_rx (
        .clk(sys), .rst_n(rst_n),
        .div(16'd0),
        // Eight data bits, even parity: ISO 7816's character is 8E2, and
        // the receiver reads only the first stop bit so the guard time
        // needs no configuring here.
        .cfg_data_bits(4'd8),
        .cfg_parity(3'd2),
        .rx(io_sync[1]),
        .rx_ready(1'b1),
        .rx_data(card_byte), .rx_valid(card_valid),
        .rx_frame_error(card_frame_err), .rx_parity_error(card_parity_err),
        .rx_break(), .rx_overrun());

    // =================================================================
    // The sequencer
    // =================================================================

    localparam [2:0] S_IDLE = 3'd0, S_VCC = 3'd1, S_CLK = 3'd2,
                     S_RUN  = 3'd3, S_DOWN = 3'd4, S_OFF = 3'd5;

    reg [2:0]         state = S_IDLE;
    reg [VCC_BITS:0]  settle = {(VCC_BITS + 1){1'b0}};
    reg [15:0]        held  = 16'd0;
    reg               vcc_q = 1'b0;
    reg               rst_q = 1'b0;

    // Which banner to print, if any.
    reg [2:0] banner = 3'd0;   // 0 none, 1 +VCC, 2 +CLK, 3 +RST, 4 -OFF

    // **A watchdog, because a device must never be left powered.** It
    // reloads on any serial command and on any character from the card, and
    // when it expires the sequencer deactivates exactly as `D` would. So a
    // crashed host, a closed terminal or a forgotten session cannot leave
    // the relay on; the worst case is `WDOG_BITS` of system clocks.
    //
    // 2^31 at 112 MHz is 19 seconds. Long enough that a slow card or a
    // human typing is never cut off, short enough that nothing is left
    // powered for long.
    reg [WDOG_BITS:0] wdog = {(WDOG_BITS + 1){1'b0}};
    wire wdog_bite = (state == S_RUN || state == S_VCC || state == S_CLK)
                   && (wdog == 0);

    always @(posedge sys) begin
        if (!rst_n) wdog <= {(WDOG_BITS + 1){1'b0}};
        else if (cmd_valid || card_valid) wdog <= {1'b1, {WDOG_BITS{1'b0}}};
        else if (wdog != 0) wdog <= wdog - 1'b1;
    end

    always @(posedge sys) begin
        if (!rst_n) begin
            state  <= S_IDLE;
            vcc_q  <= 1'b0;
            rst_q  <= 1'b0;
            clk_on <= 1'b0;
            settle <= {(VCC_BITS + 1){1'b0}};
            held   <= 16'd0;
            banner <= 3'd0;
        end else begin
            // **A pulse, not a level.** Held high, it would be latched
            // again the cycle after the printer consumed it, and the probe
            // would print `+VCC` forever without ever advancing. The
            // transitions below override this default.
            banner <= 3'd0;
            case (state)
                S_IDLE:
                    if (cmd_valid && cmd_data == 8'h41) begin   // 'A'
                        vcc_q  <= 1'b1;
                        settle <= {1'b1, {VCC_BITS{1'b0}}};
                        banner <= 3'd1;
                        state  <= S_VCC;
                    end
                S_VCC:
                    if (wdog_bite) begin
                        vcc_q  <= 1'b0;
                        clk_on <= 1'b0;
                        banner <= 3'd4;
                        state  <= S_OFF;
                    end else if (settle != 0) settle <= settle - 1'b1;
                    else begin
                        clk_on <= 1'b1;
                        held   <= 16'd0;
                        banner <= 3'd2;
                        state  <= S_CLK;
                    end
                S_CLK:
                    if (wdog_bite) begin
                        rst_q  <= 1'b0;
                        clk_on <= 1'b0;
                        vcc_q  <= 1'b0;
                        banner <= 3'd4;
                        state  <= S_OFF;
                    end else if (crise) begin
                        if (held == RST_HOLD - 1) begin
                            rst_q  <= 1'b1;        // released
                            banner <= 3'd3;
                            state  <= S_RUN;
                        end else begin
                            held <= held + 16'd1;
                        end
                    end
                S_RUN:
                    if ((cmd_valid && cmd_data == 8'h44) || wdog_bite) begin
                        rst_q  <= 1'b0;
                        clk_on <= 1'b0;
                        settle <= {1'b1, {VCC_BITS{1'b0}}};
                        state  <= S_DOWN;
                    end
                S_DOWN:
                    // Reset low and the clock stopped; wait before cutting
                    // power so nothing is driven into a dying supply.
                    if (settle != 0) settle <= settle - 1'b1;
                    else begin
                        vcc_q  <= 1'b0;
                        banner <= 3'd4;
                        state  <= S_OFF;
                    end
                default:
                    state <= S_IDLE;
            endcase
        end
    end

    assign vcc_en   = vcc_q;
    assign rst_card = rst_q;

    // =================================================================
    // Counters
    // =================================================================

    reg [15:0] bytes_q = 16'd0, perr_q = 16'd0, ferr_q = 16'd0;
    always @(posedge sys) begin
        if (!rst_n) begin
            bytes_q <= 16'd0; perr_q <= 16'd0; ferr_q <= 16'd0;
        end else if (card_valid) begin
            if (bytes_q != 16'hFFFF) bytes_q <= bytes_q + 16'd1;
            if (card_parity_err && perr_q != 16'hFFFF) perr_q <= perr_q + 16'd1;
            if (card_frame_err  && ferr_q != 16'hFFFF) ferr_q <= ferr_q + 16'd1;
        end
    end

    // =================================================================
    // Printing
    // =================================================================
    //
    // A tiny queue of what to say next, because three things can want the
    // port at once: a banner, a byte from the card, and the end of a line.

    localparam [2:0] P_IDLE = 3'd0, P_BANNER = 3'd1, P_BYTE = 3'd2,
                     P_EOL  = 3'd3, P_STATUS = 3'd4;

    reg [2:0]  phase = P_IDLE;
    reg [3:0]  pos   = 4'd0;
    reg [7:0]  pend_byte = 8'd0;
    reg        pend_bad  = 1'b0;
    reg        pend_frame = 1'b0;
    reg        have_byte = 1'b0;
    // **One pending bit per banner, not one slot for all of them.** The
    // four happen in a fixed order, and with a single slot a banner that
    // arrives while another is still going out is simply lost: `+VCC` takes
    // about 960 cycles to print at a short divisor and the settle can be
    // shorter than that, so `+CLK` vanished and a host could not tell how
    // far activation had got. On the board the settle is a million cycles
    // and this would never have shown — a bug that only appears in the
    // field, which is the worst kind to leave in.
    //
    // Priority is the order they occur in, so they can never come out
    // reordered even if two are pending at once.
    reg p_vcc = 1'b0, p_clk = 1'b0, p_rst = 1'b0, p_off = 1'b0;
    wire [2:0] want_banner = p_vcc ? 3'd1
                           : p_clk ? 3'd2
                           : p_rst ? 3'd3
                           : p_off ? 3'd4
                           :         3'd0;
    reg        want_status = 1'b0;
    reg [127:0] status_sh = 128'd0;
    reg [5:0]  status_left = 6'd0;

    // The line is over when nothing has arrived for a while.
    reg [31:0] gap = 32'd0;
    reg              line_open = 1'b0;

    // **Finished, not started.** Each of these says the last character of
    // its message has just been taken by the transmitter. Clearing a
    // pending flag when printing *begins* is wrong twice over: the rest of
    // the message then reads a cleared `want_banner` and prints the
    // function's default, and a message that does not start at position
    // zero — a byte with no error marker starts at one — never clears its
    // flag at all and repeats for ever. Both of those happened.
    wire banner_done = (phase == P_BANNER) && (pos == 4'd5) && tx_valid && tx_ready;
    wire byte_done   = (phase == P_BYTE)   && (pos == 4'd2) && tx_valid && tx_ready;
    wire eol_done    = (phase == P_EOL)    && (pos == 4'd1) && tx_valid && tx_ready;

    always @(posedge sys) begin
        if (!rst_n) begin
            have_byte <= 1'b0;
            gap       <= 32'd0;
            line_open <= 1'b0;
        end else begin
            if (card_valid) begin
                pend_byte  <= card_byte;
                pend_bad   <= card_parity_err;
                pend_frame <= card_frame_err;
                have_byte  <= 1'b1;
                gap        <= GAP_LOAD;
                line_open  <= 1'b1;
            end else if (gap != 0) begin
                gap <= gap - 1'b1;
            end
            if (byte_done) have_byte <= 1'b0;
            if (eol_done)  line_open <= 1'b0;
        end
    end

    always @(posedge sys) begin
        if (!rst_n) begin
            p_vcc <= 1'b0; p_clk <= 1'b0; p_rst <= 1'b0; p_off <= 1'b0;
        end else begin
            if (banner == 3'd1) p_vcc <= 1'b1;
            if (banner == 3'd2) p_clk <= 1'b1;
            if (banner == 3'd3) p_rst <= 1'b1;
            if (banner == 3'd4) p_off <= 1'b1;
            if (banner_done) begin
                case (want_banner)
                    3'd1: p_vcc <= 1'b0;
                    3'd2: p_clk <= 1'b0;
                    3'd3: p_rst <= 1'b0;
                    default: p_off <= 1'b0;
                endcase
            end
        end
    end

    always @(posedge sys) begin
        if (!rst_n) want_status <= 1'b0;
        else if (cmd_valid && cmd_data == 8'h73) want_status <= 1'b1;  // 's'
        else if (phase == P_STATUS && status_left == 0) want_status <= 1'b0;
    end

    function [7:0] hex;
        input [3:0] n;
        hex = (n < 4'd10) ? (8'd48 + {4'd0, n}) : (8'd55 + {4'd0, n});
    endfunction

    // Four banner texts, four characters each, chosen the same length so
    // one index walks all of them.
    function [7:0] banner_ch;
        input [2:0] which;
        input [1:0] at;
        case ({which, at})
            {3'd1, 2'd0}: banner_ch = 8'h2B;  // +
            {3'd1, 2'd1}: banner_ch = "V";
            {3'd1, 2'd2}: banner_ch = "C";
            {3'd1, 2'd3}: banner_ch = "C";
            {3'd2, 2'd0}: banner_ch = 8'h2B;
            {3'd2, 2'd1}: banner_ch = "C";
            {3'd2, 2'd2}: banner_ch = "L";
            {3'd2, 2'd3}: banner_ch = "K";
            {3'd3, 2'd0}: banner_ch = 8'h2B;
            {3'd3, 2'd1}: banner_ch = "R";
            {3'd3, 2'd2}: banner_ch = "S";
            {3'd3, 2'd3}: banner_ch = "T";
            {3'd4, 2'd0}: banner_ch = 8'h2D;  // -
            {3'd4, 2'd1}: banner_ch = "O";
            {3'd4, 2'd2}: banner_ch = "F";
            default:      banner_ch = "F";
        endcase
    endfunction

    wire [127:0] status = {
        bytes_q, perr_q, ferr_q, ETU_DIV16,
        CARD_DIV16, RST_HOLD16,
        {5'd0, state}, 8'hA5
    };

    always @(posedge sys) begin
        if (!rst_n) begin
            phase    <= P_IDLE;
            pos      <= 4'd0;
            tx_valid <= 1'b0;
        end else if (!tx_valid) begin
            case (phase)
                P_IDLE: begin
                    if (want_banner != 3'd0) begin
                        phase <= P_BANNER; pos <= 4'd0;
                    end else if (want_status) begin
                        status_sh   <= status;
                        status_left <= 6'd32;
                        phase       <= P_STATUS;
                    end else if (have_byte) begin
                        phase <= P_BYTE;
                        pos   <= (pend_bad || pend_frame) ? 4'd0 : 4'd1;
                    end else if (line_open && gap == 0) begin
                        phase <= P_EOL; pos <= 4'd0;
                    end
                end
                P_BANNER: begin
                    tx_data  <= (pos < 4'd4) ? banner_ch(want_banner, pos[1:0])
                              : (pos == 4'd4) ? 8'd13 : 8'd10;
                    tx_valid <= 1'b1;
                end
                P_BYTE: begin
                    tx_data  <= (pos == 4'd0) ? (pend_bad ? 8'h21 : 8'h3F)
                              : (pos == 4'd1) ? hex(pend_byte[7:4])
                              :                 hex(pend_byte[3:0]);
                    tx_valid <= 1'b1;
                end
                P_EOL: begin
                    tx_data  <= (pos == 4'd0) ? 8'd13 : 8'd10;
                    tx_valid <= 1'b1;
                end
                default: begin
                    tx_data  <= (status_left != 0) ? hex(status_sh[127:124])
                              : (status_left == 0 && pos == 4'd0) ? 8'd13
                              :                                     8'd10;
                    tx_valid <= 1'b1;
                end
            endcase
        end else if (tx_ready) begin
            tx_valid <= 1'b0;
            case (phase)
                P_BANNER:
                    if (pos == 4'd5) begin phase <= P_IDLE; pos <= 4'd0; end
                    else                   pos   <= pos + 4'd1;
                P_BYTE:
                    if (pos == 4'd2) begin phase <= P_IDLE; pos <= 4'd0; end
                    else                   pos   <= pos + 4'd1;
                P_EOL:
                    if (pos == 4'd1) begin phase <= P_IDLE; pos <= 4'd0; end
                    else                   pos   <= pos + 4'd1;
                P_STATUS:
                    if (status_left != 0) begin
                        status_sh   <= {status_sh[123:0], 4'd0};
                        status_left <= status_left - 6'd1;
                    end else if (pos == 4'd1) begin
                        phase <= P_IDLE; pos <= 4'd0;
                    end else begin
                        pos <= pos + 4'd1;
                    end
                default: phase <= P_IDLE;
            endcase
        end
    end

    // =================================================================
    // The board's own display
    // =================================================================

    reg [25:0] tick = 26'd0;
    always @(posedge sys) tick <= tick + 26'd1;

    assign led = {tick[25], perr_q != 0, ferr_q != 0, pll_locked,
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
