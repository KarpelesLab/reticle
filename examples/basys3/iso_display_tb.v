// `iso_display.v` with both halves working at once: a model card on the
// contact, real SSD1306 traffic on the four SPI pins, and one serial port
// carrying both conversations.
//
// It is the two existing testbenches joined at the port.
// `iso7816_terminal_tb.v` supplies the card side — a second `iso7816_uart`
// as the model card, on one open-drain wire modelled as the wired-AND a
// pull-up physically is. `ssd1306_console_tb.v` supplies the display side —
// an initialisation sequence and a frame driven bit by bit, then read back
// and compared with what was drawn. Neither idiom is reinvented here.
//
// What it checks, beyond what those two already did:
//
//   1. **The two halves on one port.** A dump is asked for while the card
//      is talking, so the arbiter has to interleave them. Every line that
//      comes out is read whole and classified, and a dump page must be
//      exactly `2 * COLUMNS` characters with every byte equal to what was
//      drawn. A single character of one half inside the other's line would
//      make the page the wrong length, or its bytes wrong, or both.
//   2. **The alphabets do not collide.** An ISO command produces exactly
//      one line and then silence — the display half used to answer anything
//      it did not recognise, which on a shared port would bury the card.
//      A `:` run of hexadecimal reaches the card and presses no button.
//   3. **The buttons**, which no testbench has looked at before: the core
//      asks for a press rather than driving a ball, so `press_left` and
//      `press_right` are plain wires here, and both are checked to rise on
//      their character and fall again on their own.
//
// What it would not catch: anything about the pads or the PLL, which live
// in `iso_display_pad.v` and are not in this simulation; the real card's
// timing tolerances; metastability; the real `sclk` rate; or whether the
// SSD1306 command set as implemented matches a real panel.
//
// ===================================================================
// WHAT IS SCALED, AND AGAINST WHAT
// ===================================================================
//
// Five of this directory's testbench bugs were one mistake: a constant
// scaled for simulation while the constant next to it kept its board value,
// breaking a relationship the design depends on. So each one here says what
// it is measured in and what it has to stay bigger or smaller than.
//
//   ETU_CYCLES = 24          **card clocks per etu**, 372 on the board. The
//                            contact's bit time. Everything below that is
//                            "in etu" follows it automatically.
//   FAST_ETU_CYCLES = 4      the same after PPS, and the board's real
//                            value. The ratio here is 6:1 where the board's
//                            is 93:1; what the test needs is that the
//                            change is abrupt and that both ends make it.
//   RST_HOLD = 400           **card clocks**, the board's own value: it
//                            costs 400 * CARD_DIV = 5600 system cycles,
//                            which is affordable unscaled.
//   VCC_BITS = 6             a power settling delay in **system clocks**,
//                            2**N. 20 on the board is 9 ms.
//   GAP_ETU = 24             **etu** of silence that close a printed line.
//                            It is in etu, so it scales with ETU_CYCLES —
//                            and it must stay **larger** than the 14 etu
//                            `card_send` leaves between bytes, or the
//                            model card's own answer would arrive as
//                            several lines instead of one.
//   WDOG_BITS = 24           the idle watchdog in **system clocks**, 2**N,
//                            31 on the board. It must outlast the longest
//                            stretch of this test with no host and no card
//                            byte, which is the SPI frame — a few thousand
//                            cycles against sixteen million.
//   HOST_DIV = 16            **system clocks per serial bit**. 972 on the
//                            board is 115200 baud at 112 MHz.
//   PRESS_BITS = 6           a button hold in **system clocks**, 2**N, 23
//                            on the board. Scaled right down so that the
//                            press ends inside this test and the release
//                            can be checked.
//   TICK_BIT = 10            the heartbeat and SHOW_BIT = 8 the digits'
//   SHOW_BIT = 8             alternation, both **system clocks**, 2**N, and
//                            both only drive lamps and glass. Nothing here
//                            asserts on either; they are scaled down so the
//                            counters they belong to wrap often enough to
//                            have been exercised at all.
//   COLUMNS x PAGES = 16x2   the display geometry. The RTL is the same at
//                            128x8; a full dump there is 2064 characters
//                            and some hundreds of thousands of cycles for
//                            one check, and the wrap, the frame strobe, the
//                            line breaks and the hex emitter are exercised
//                            just as well by 32 bytes.

`timescale 1ns / 1ps

module iso_display_tb;
    localparam integer CARD_DIV        = 14;
    localparam integer ETU_CYCLES      = 24;
    localparam integer FAST_ETU_CYCLES = 4;
    localparam integer SLOW_DIV        = ETU_CYCLES * CARD_DIV;
    localparam integer FAST_DIV        = FAST_ETU_CYCLES * CARD_DIV;
    localparam integer HOST_DIV        = 16;
    localparam integer RST_HOLD        = 400;
    localparam integer GAP_ETU         = 24;
    localparam integer PRESS_BITS      = 6;
    localparam integer COLUMNS         = 16;
    localparam integer PAGES           = 2;
    localparam integer WORDS           = COLUMNS * PAGES;
    localparam integer PHASE           = 5;    // system clocks per sclk half
    localparam [7:0]   COL_LAST        = COLUMNS - 1;
    localparam [7:0]   PAGE_LAST       = PAGES - 1;

    reg clk = 1'b0;
    always #5 clk = ~clk;

    // The shared open-drain contact, as the **wired-AND** a pull-up
    // physically is. Not two tristate drivers: `reticle sim` resolves those
    // but has no pull-up, so a released line would read `z` and prove
    // nothing. Each side either pulls low or releases, so the line is low
    // exactly when somebody pulls it. This is why the tristate lives in
    // `iso_display_pad.v` and the core takes `io_i`.
    wire term_oe, term_o, card_oe, card_o;
    wire io = ~((term_oe & ~term_o) | (card_oe & ~card_o));

    reg  sclk = 1'b0, mosi = 1'b0, dc = 1'b0, cs_n = 1'b1;

    wire clk_card, rst_card, vcc_en, dut_tx;
    wire press_left, press_right;
    wire [14:0] led;
    wire [6:0]  seg;
    wire        dp;
    wire [3:0]  an;
    wire        host_tx;

    iso_display #(
        .CARD_DIV(CARD_DIV), .ETU_CYCLES(ETU_CYCLES),
        .FAST_ETU_CYCLES(FAST_ETU_CYCLES), .RST_HOLD(RST_HOLD),
        .VCC_BITS(6), .GAP_ETU(GAP_ETU), .WDOG_BITS(24),
        .HOST_DIV(HOST_DIV), .TICK_BIT(10),
        .COLUMNS(COLUMNS), .PAGES(PAGES), .PRESS_BITS(PRESS_BITS),
        .SHOW_BIT(8)
    ) dut (
        .clk(clk), .locked(1'b1),
        .clk_card(clk_card), .rst_card(rst_card), .vcc_en(vcc_en),
        .io_i(io), .io_oe(term_oe), .io_o(term_o),
        .sclk(sclk), .mosi(mosi), .dc(dc), .cs_n(cs_n),
        .press_left(press_left), .press_right(press_right),
        .buttons_in(2'b00),
        .uart_rx_pin(host_tx), .uart_tx_pin(dut_tx),
        .led(led), .seg(seg), .dp(dp), .an(an));

    // ---- The model card: the same IP block, the other way round ----
    reg         card_rst_n = 1'b0;
    reg  [15:0] card_etu   = SLOW_DIV;
    reg  [7:0]  card_tx    = 8'd0;
    reg         card_tx_v  = 1'b0;
    wire        card_tx_rdy;
    wire [7:0]  card_rx;
    wire        card_rx_v, card_perr;
    iso7816_uart #(.ETU_DIV(SLOW_DIV)) card (
        .clk(clk), .rst_n(card_rst_n),
        .active(1'b1),
        .etu_div(card_etu), .guard_etu(8'd0), .convention(1'b0),
        .wt_etu(24'd9600),
        .tx_data(card_tx), .tx_valid(card_tx_v), .tx_ready(card_tx_rdy),
        .tx_abort(),
        .rx_data(card_rx), .rx_ready(1'b1), .rx_valid(card_rx_v),
        .rx_parity_error(card_perr), .rx_overrun(), .rx_timeout(),
        .io_i(io), .io_oe(card_oe), .io_o(card_o),
        .tx_char_count(), .rx_char_count(), .parity_error_count(),
        .repeat_count(), .timeout_count());

    // The card only wakes when the terminal releases reset.
    always @(posedge clk) card_rst_n <= rst_card;

    // ---- Everything the card received, so a frame out can be checked ----
    reg [7:0] got [0:63];
    integer   got_n = 0;
    always @(posedge clk) begin
        if (card_rx_v) begin
            got[got_n % 64] = card_rx;
            got_n = got_n + 1;
        end
    end

    // **Bounded, and it says why it gave up.** An unbounded wait makes
    // every failure read "did not finish in the time allowed", which names
    // the wrong subsystem: the terminal deactivating on its own watchdog
    // because this task stalled looks nothing like the cause.
    integer waited;
    task card_send;
        input [7:0] v;
        begin
            card_tx   = v;
            card_tx_v = 1'b1;
            waited    = 0;
            while (!(card_tx_v && card_tx_rdy)) begin
                @(posedge clk);
                waited = waited + 1;
                if (waited > card_etu * 40) begin
                    $display("FAIL: the model card never became ready to send %02x; its parity-error flag reads %b and the terminal is %s",
                             v, card_perr, rst_card ? "running" : "in reset");
                    $finish;
                end
            end
            @(posedge clk);
            card_tx_v = 1'b0;
            // Fourteen etu before the next character. Two things depend on
            // this number: it must be **more** than the guard time the
            // terminal samples for T=0's error pulse in (answering
            // instantly makes a start bit read as that pulse, which cost
            // several rounds), and **less** than GAP_ETU, or each byte
            // would end up on a line of its own.
            repeat (card_etu * 14) @(posedge clk);
        end
    endtask

    // Anything the card thinks is malformed, reported as it happens.
    always @(posedge clk) begin
        if (card_rst_n && card_perr)
            $display("note: at %0t the model card flagged a parity error", $time);
    end

    // ---- The host's end of the one serial port ----
    reg host_rst_n = 1'b0;
    initial begin
        repeat (8) @(posedge clk);
        host_rst_n = 1'b1;
    end

    reg  [7:0] host_data  = 8'd0;
    reg        host_valid = 1'b0;
    wire       host_ready;
    uart_tx #(.CLK_DIV(HOST_DIV)) host_out (
        .clk(clk), .rst_n(host_rst_n), .div(16'd0),
        .tx_data(host_data), .tx_valid(host_valid), .tx_ready(host_ready),
        .tx(host_tx));

    wire [7:0] from_dut;
    wire       from_dut_valid;
    uart_rx #(.CLK_DIV(HOST_DIV)) host_in (
        .clk(clk), .rst_n(host_rst_n), .div(16'd0),
        .rx(dut_tx), .rx_data(from_dut), .rx_valid(from_dut_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

    // Buffered continuously: a testbench that only polls the strobe when
    // convenient loses every character that arrives while it is talking,
    // and here it is talking to the card and the display at once.
    reg [7:0] rxq [0:4095];
    integer   rx_wr = 0, rx_rd = 0;
    always @(posedge clk) begin
        if (from_dut_valid) begin
            rxq[rx_wr % 4096] = from_dut;
            rx_wr = rx_wr + 1;
        end
    end

    reg [7:0] ch;
    task host_recv;
        begin
            while (rx_rd >= rx_wr) @(posedge clk);
            ch    = rxq[rx_rd % 4096];
            rx_rd = rx_rd + 1;
        end
    endtask

    task host_send;
        input [7:0] c;
        begin
            host_data  = c;
            host_valid = 1'b1;
            while (!(host_valid && host_ready)) @(posedge clk);
            @(posedge clk);
            host_valid = 1'b0;
            repeat (HOST_DIV * 12) @(posedge clk);
        end
    endtask

    // One byte of a `:` run: two hex characters, then a pause.
    //
    // **The terminal takes one byte at a time.** Its `host_byte_ready` is a
    // one-cycle pulse, and a byte offered while a character is still going
    // out on the contact is dropped silently — so a host has to pace a run,
    // and this is how. Twenty **etu** is the pause, and it has a window at
    // both ends: more than the twelve etu a character costs, or a byte is
    // lost, and less than the GAP_ETU = 24 that closes a printed line, or
    // the four bytes the monitor reads back would arrive as four lines
    // instead of one. On the board the pause is free at the fast rate — two
    // hex characters take 174 us at 115200 baud against 5.5 us for a card
    // character at 2 Mbaud — and needs care at the slow one.
    task card_byte_out;
        input [7:0] v;
        begin
            host_send(hexch(v[7:4]));
            host_send(hexch(v[3:0]));
            repeat (card_etu * 20) @(posedge clk);
        end
    endtask

    // Bounded, and it says what it saw: an unbounded wait here would make
    // a dropped byte read "did not finish in the time allowed".
    integer card_waited;
    task wait_card_bytes;
        input integer n;
        begin
            card_waited = 0;
            while (got_n < n) begin
                @(posedge clk);
                card_waited = card_waited + 1;
                if (card_waited > card_etu * 40 * n) begin
                    $display("FAIL: the card received %0d of %0d byte(s), the first %02x. A byte offered to the terminal while a character is still on the contact is dropped.",
                             got_n, n, got[0]);
                    $finish;
                end
            end
        end
    endtask

    // ---- The far side of the SPI link ----
    //
    // One byte per `cs_n`, most significant bit first, sampled on the
    // rising edge of `sclk`, with `dc` held for the whole frame, which is
    // what a display controller does.
    integer b;
    task spi_byte;
        input [7:0] value;
        input       is_data;
        begin
            dc   = is_data;
            cs_n = 1'b0;
            repeat (PHASE) @(posedge clk);
            for (b = 7; b >= 0; b = b - 1) begin
                mosi = value[b];
                repeat (PHASE) @(posedge clk);
                sclk = 1'b1;
                repeat (PHASE) @(posedge clk);
                sclk = 1'b0;
            end
            repeat (PHASE) @(posedge clk);
            cs_n = 1'b1;
            repeat (PHASE * 2) @(posedge clk);
        end
    endtask

    task cmd; input [7:0] v; begin spi_byte(v, 1'b0); end endtask
    task dat; input [7:0] v; begin spi_byte(v, 1'b1); end endtask

    // ---- Hex ----
    function is_hex;
        input [7:0] c;
        is_hex = (c >= 8'd48 && c <= 8'd57) || (c >= 8'd65 && c <= 8'd70);
    endfunction

    function [3:0] nib;
        input [7:0] c;
        nib = (c <= 8'd57) ? (c - 8'd48) : (c - 8'd55);
    endfunction

    function [7:0] hexch;
        input [3:0] n;
        hexch = (n < 4'd10) ? (8'd48 + {4'd0, n}) : (8'd55 + {4'd0, n});
    endfunction

    // ---- Reading whole lines ----
    //
    // The arbiter's promise is about lines, so the check is about lines: a
    // line is read to its CRLF and then judged as a whole. Leading line
    // endings are skipped, because a line can always follow the one
    // something else closed — hand-counting those has broken the testbench
    // next door three times.
    reg [7:0] line_buf [0:4095];
    integer   line_len;
    task read_line;
        begin
            line_len = 0;
            host_recv;
            while (ch === 8'd13 || ch === 8'd10) host_recv;
            while (ch !== 8'd13) begin
                line_buf[line_len] = ch;
                line_len = line_len + 1;
                host_recv;
            end
            host_recv;
            if (ch !== 8'd10) begin
                $display("FAIL: a line ended CR then %02x, wanted LF", ch);
                $finish;
            end
        end
    endtask

    task expect_banner;
        input [8*4-1:0] want;
        integer k;
        begin
            read_line;
            if (line_len !== 4) begin
                $display("FAIL: wanted the banner '%c%c%c%c', got a line of %0d characters",
                         want[31:24], want[23:16], want[15:8], want[7:0], line_len);
                $finish;
            end
            for (k = 0; k < 4; k = k + 1) begin
                if (line_buf[k] !== want[8*(3-k) +: 8]) begin
                    $display("FAIL: banner character %0d is '%c' (%02x), wanted '%c'",
                             k, line_buf[k], line_buf[k], want[8*(3-k) +: 8]);
                    $finish;
                end
            end
        end
    endtask

    // A line of card bytes: two hex characters each, no prefix.
    // `n` bytes are expected, in `want` packed most significant first.
    task expect_card_line;
        input integer n;
        input [8*16-1:0] want;
        integer k;
        reg [7:0] v;
        begin
            read_line;
            if (line_len !== n * 2) begin
                $display("FAIL: wanted %0d card byte(s), got a line of %0d characters",
                         n, line_len);
                $finish;
            end
            for (k = 0; k < n; k = k + 1) begin
                v = want[8*(n-1-k) +: 8];
                if (line_buf[2*k] !== hexch(v[7:4])
                    || line_buf[2*k+1] !== hexch(v[3:0])) begin
                    $display("FAIL: card byte %0d printed '%c%c', wanted %02x",
                             k, line_buf[2*k], line_buf[2*k+1], v);
                    $finish;
                end
            end
        end
    endtask

    // Bytes the monitor saw, which are the ones this terminal transmitted:
    // `>` then two hex characters each, all on one line. A `!` instead of
    // the `>` means the parity was wrong on the wire, which is worth
    // reporting as itself rather than as a mismatch.
    task expect_monitor_line;
        input integer n;
        input [8*16-1:0] want;
        integer k;
        reg [7:0] v;
        begin
            read_line;
            if (line_len !== n * 3) begin
                $display("FAIL: wanted %0d monitored byte(s) (%0d characters), got %0d",
                         n, n * 3, line_len);
                $finish;
            end
            for (k = 0; k < n; k = k + 1) begin
                v = want[8*(n-1-k) +: 8];
                if (line_buf[3*k] === 8'h21) begin
                    $display("FAIL: the monitor flagged bad parity on byte %0d of what this terminal sent",
                             k);
                    $finish;
                end
                if (line_buf[3*k] !== 8'h3E) begin
                    $display("FAIL: wanted `>` before monitored byte %0d, got '%c'",
                             k, line_buf[3*k]);
                    $finish;
                end
                if (line_buf[3*k+1] !== hexch(v[7:4])
                    || line_buf[3*k+2] !== hexch(v[3:0])) begin
                    $display("FAIL: monitored byte %0d printed '%c%c', wanted %02x",
                             k, line_buf[3*k+1], line_buf[3*k+2], v);
                    $finish;
                end
            end
        end
    endtask

    // Nothing more is coming. Measured in characters' worth of serial time
    // rather than in anything absolute: four characters at this divisor is
    // long enough for a line either half had begun to have reached here.
    task expect_silence;
        input [8*16-1:0] what;
        begin
            repeat (HOST_DIV * 10 * 4) @(posedge clk);
            if (rx_rd < rx_wr) begin
                $display("FAIL: %0s, but %0d character(s) arrived, the first '%c' (%02x)",
                         what, rx_wr - rx_rd, rxq[rx_rd % 4096], rxq[rx_rd % 4096]);
                $finish;
            end
        end
    endtask

    // ---- A card byte sent while the dump is going out ----
    //
    // Its own process, because the point is that it happens *during* the
    // dump rather than before or after it. One byte and not two: the card
    // half holds exactly one received byte, and a second arriving before
    // the first has reached the port would overwrite it — which is the
    // documented cost of sharing the port and not something to assert
    // against.
    reg burst      = 1'b0;
    reg burst_done = 1'b0;
    initial begin
        while (!burst) @(posedge clk);
        card_send(8'hC3);
        burst_done = 1'b1;
    end

    // What was drawn, so the dump is compared with something this file
    // computed rather than with the device's own idea of it.
    reg [7:0] drew [0:WORDS-1];

    integer i, p, x, pages_seen, card_seen, deact;
    reg [7:0] got_byte, want;
    reg [7:0] atr [0:13];

    initial begin
        #800_000_000;
        $display("FAIL: did not finish in the time allowed");
        $finish;
    end

    initial begin
        repeat (64) @(posedge clk);
        if (vcc_en !== 1'b0) begin
            $display("FAIL: powered before being asked"); $finish;
        end
        if (press_left !== 1'b0 || press_right !== 1'b0) begin
            $display("FAIL: a button is pressed before being asked"); $finish;
        end

        // =============================================================
        // 1. The card half, on its own
        // =============================================================

        host_send(8'h41);                      // 'A'
        expect_banner("+VCC");
        expect_banner("+CLK");
        expect_banner("+RST");

        // The real device's answer-to-reset, measured off the part.
        atr[0]  = 8'h3B; atr[1]  = 8'h1B; atr[2]  = 8'h87; atr[3]  = 8'h05;
        atr[4]  = 8'h32; atr[5]  = 8'h2E; atr[6]  = 8'h35; atr[7]  = 8'h2E;
        atr[8]  = 8'h31; atr[9]  = 8'h04; atr[10] = 8'h33; atr[11] = 8'h00;
        atr[12] = 8'h00; atr[13] = 8'h04;
        for (i = 0; i < 14; i = i + 1) card_send(atr[i]);

        // One line, because `card_send` waits 14 etu and GAP_ETU is 24.
        read_line;
        if (line_len !== 28) begin
            $display("FAIL: the ATR came back as a line of %0d characters, wanted 28",
                     line_len);
            $finish;
        end
        for (i = 0; i < 14; i = i + 1) begin
            // Through an eight-bit local: a bit-select of an array element
            // is not a construct this frontend takes.
            want = atr[i];
            if (line_buf[2*i] !== hexch(want[7:4])
                || line_buf[2*i+1] !== hexch(want[3:0])) begin
                $display("FAIL: ATR byte %0d printed '%c%c', wanted %02x",
                         i, line_buf[2*i], line_buf[2*i+1], want);
                $finish;
            end
        end

        // =============================================================
        // 2. A frame out, at the slow rate, and the display half silent
        // =============================================================
        //
        // `:00A4040C` is four bytes to the card. Two things are being
        // checked at once here. The first is that a run's characters are
        // **data in both halves**: the card half gates every command
        // decoder on its own `hex_run`, and the router gates the display
        // half with the same bit. The second is the defect that found this
        // test rather than the other way round — every command decoder used
        // to read the byte alone, so the `C` of `040C` also ran the `C`
        // command, which drives the contact low for 18 ms, and the card
        // received `00 A4 04 00`. A `D` nibble would have deactivated it.
        got_n = 0;
        host_send(8'h3A);                      // ':'
        card_byte_out(8'h00);
        card_byte_out(8'hA4);
        card_byte_out(8'h04);
        card_byte_out(8'h0C);
        host_send(8'h0D);                      // CR closes the run
        wait_card_bytes(4);
        if (got[0] !== 8'h00 || got[1] !== 8'hA4
            || got[2] !== 8'h04 || got[3] !== 8'h0C) begin
            $display("FAIL: the card received %02x %02x %02x %02x, wanted 00 A4 04 0C",
                     got[0], got[1], got[2], got[3]);
            $finish;
        end
        // The monitor decoded all four off the wire, which is the evidence
        // that what went out was well formed rather than merely counted.
        expect_monitor_line(4, {96'd0, 8'h00, 8'hA4, 8'h04, 8'h0C});
        expect_silence("a `:` run and a CR are the card half's business alone");

        // And a run closed by a display command does not run it: `g` is not
        // a hex digit, so it ends the run, and it is still inside the run on
        // the cycle it arrives, so the display half never sees it. Without
        // the gate this would dump the frame buffer — which at this point is
        // still blank, so the failure would be eight lines of zeros.
        host_send(8'h3A);                      // ':'
        host_send(8'h67);                      // 'g' closes it and does nothing
        expect_silence("a `:` run's terminator belongs to the card half");

        // =============================================================
        // 3. The display half: a real initialisation and a frame
        // =============================================================

        cmd(8'hAE);                     // display off
        cmd(8'h20); cmd(8'h00);         // horizontal addressing
        cmd(8'h8D); cmd(8'h14);         // charge pump
        cmd(8'h81); cmd(8'hCF);         // contrast
        cmd(8'hAF);                     // display on
        cmd(8'h21); cmd(8'h00); cmd(COL_LAST);
        cmd(8'h22); cmd(8'h00); cmd(PAGE_LAST);

        // Bytes no off-by-one could reproduce.
        for (p = 0; p < PAGES; p = p + 1) begin
            for (x = 0; x < COLUMNS; x = x + 1) begin
                drew[p * COLUMNS + x] = {p[3:0], x[3:0]} ^ 8'h5A;
                dat(drew[p * COLUMNS + x]);
            end
        end
        expect_silence("SPI traffic is not something either half announces");

        // =============================================================
        // 4. Both halves at once: a dump, with the card talking into it
        // =============================================================

        host_send(8'h67);               // 'g', the dump
        burst = 1'b1;                   // and the card speaks into it

        pages_seen = 0;
        card_seen  = 0;
        while (pages_seen < PAGES || card_seen == 0) begin
            read_line;
            if (line_len == COLUMNS * 2) begin
                // A page of the dump. Whole, in order, and equal to what
                // was drawn — a character of the card half anywhere inside
                // it would change the length or the bytes or both.
                for (x = 0; x < COLUMNS; x = x + 1) begin
                    if (!is_hex(line_buf[2*x]) || !is_hex(line_buf[2*x+1])) begin
                        $display("FAIL: page %0d column %0d is '%c%c', not hex",
                                 pages_seen, x, line_buf[2*x], line_buf[2*x+1]);
                        $finish;
                    end
                    got_byte[7:4] = nib(line_buf[2*x]);
                    got_byte[3:0] = nib(line_buf[2*x+1]);
                    if (got_byte !== drew[pages_seen * COLUMNS + x]) begin
                        $display("FAIL: page %0d column %0d dumped %02x, drew %02x",
                                 pages_seen, x, got_byte,
                                 drew[pages_seen * COLUMNS + x]);
                        $finish;
                    end
                end
                pages_seen = pages_seen + 1;
            end else if (line_len == 2) begin
                // The card's byte, on a line of its own.
                if (line_buf[0] !== 8'h43 || line_buf[1] !== 8'h33) begin
                    $display("FAIL: a two-character line read '%c%c', wanted C3",
                             line_buf[0], line_buf[1]);
                    $finish;
                end
                card_seen = 1;
            end else begin
                $display("FAIL: a line of %0d characters arrived during the dump; the first is '%c' (%02x)",
                         line_len, line_buf[0], line_buf[0]);
                $finish;
            end
        end
        while (!burst_done) @(posedge clk);
        expect_silence("the dump and the card's byte are both finished");

        // =============================================================
        // 5. The display's status, and that an ISO command leaves it quiet
        // =============================================================

        host_send(8'h3F);               // '?'
        begin : disp_status
            // One 128-bit word rather than an array of fields: an unpacked
            // array inside a named block is not supported here.
            reg [127:0] word;
            integer j;
            read_line;
            if (line_len !== 32) begin
                $display("FAIL: the display's status was %0d characters, wanted 32",
                         line_len);
                $finish;
            end
            word = 128'd0;
            for (j = 0; j < 32; j = j + 1) begin
                if (!is_hex(line_buf[j])) begin
                    $display("FAIL: status character %0d is '%c', not hex",
                             j, line_buf[j]);
                    $finish;
                end
                word = (word << 4) | {124'd0, nib(line_buf[j])};
            end
            if (word[127:112] !== 16'd1) begin
                $display("FAIL: %0d frames completed, wanted 1", word[127:112]);
                $finish;
            end
            if (word[111:96] !== WORDS) begin
                $display("FAIL: %0d data bytes, drew %0d", word[111:96], WORDS);
                $finish;
            end
            if (word[79:64] !== 16'd0) begin
                $display("FAIL: %0d command(s) were not understood", word[79:64]);
                $finish;
            end
            if (word[63:48] !== 16'd0) begin
                $display("FAIL: the SPI receiver reported %0d bit error(s)",
                         word[63:48]);
                $finish;
            end
            // Bit 0 of the flags is always set, so an all-zero line cannot
            // be mistaken for a working link reporting an idle screen.
            if (word[24] !== 1'b1) begin
                $display("FAIL: the always-set flag bit is clear");
                $finish;
            end
        end
        expect_silence("`?` is the display half's own character");

        // The card half's status is one line, and the display half says
        // nothing about it. It used to answer every character it did not
        // recognise, which on a shared port would have doubled every
        // command's output.
        host_send(8'h73);               // 's'
        read_line;
        if (line_len !== 32) begin
            $display("FAIL: the card half's status was %0d characters, wanted 32",
                     line_len);
            $finish;
        end
        // `A5` sits at a fixed place in that word and nowhere in the
        // display's, which is how the two 32-character lines are told apart.
        // Characters 28 and 29: the four counters are sixteen of them,
        // `etu_div` and `CARD_DIV` eight more, the rate flag and the state
        // four, and then the constant.
        if (line_buf[28] !== 8'h41 || line_buf[29] !== 8'h35) begin
            $display("FAIL: the card half's status had '%c%c' where A5 belongs",
                     line_buf[28], line_buf[29]);
            $finish;
        end
        expect_silence("`s` is the card half's own character");

        // =============================================================
        // 6. The buttons, which drive no serial output at all
        // =============================================================

        host_send(8'h3C);               // '<'
        if (press_left !== 1'b1) begin
            $display("FAIL: `<` did not press the left button"); $finish;
        end
        if (press_right !== 1'b0) begin
            $display("FAIL: `<` pressed the right button too"); $finish;
        end
        // It releases on its own after 2**PRESS_BITS clocks, counted from
        // the character; twice that is a generous bound and still a count
        // of clocks, not of time.
        repeat (2 << PRESS_BITS) @(posedge clk);
        if (press_left !== 1'b0) begin
            $display("FAIL: the left button never released"); $finish;
        end

        host_send(8'h3E);               // '>'
        if (press_right !== 1'b1) begin
            $display("FAIL: `>` did not press the right button"); $finish;
        end
        repeat (2 << PRESS_BITS) @(posedge clk);
        if (press_right !== 1'b0) begin
            $display("FAIL: the right button never released"); $finish;
        end
        expect_silence("a button press is not announced");

        // =============================================================
        // 7. PPS, the rate change, and the card at 2 Mbaud
        // =============================================================

        got_n = 0;
        host_send(8'h50);                      // 'P'
        wait_card_bytes(4);
        if (got[0] !== 8'hFF || got[1] !== 8'h10
            || got[2] !== 8'h97 || got[3] !== 8'h78) begin
            $display("FAIL: the card received %02x %02x %02x %02x, wanted FF 10 97 78",
                     got[0], got[1], got[2], got[3]);
            $finish;
        end
        // **Wait before answering.** A terminal's guard time after a
        // character is when it samples for T=0's parity-error pulse, so a
        // far side that starts transmitting inside that window has its
        // start bit read as an error signal; the terminal then repeats and
        // collides with the answer. Sixteen etu, as `pair_tb` uses.
        repeat (card_etu * 16) @(posedge clk);
        for (i = 0; i < 4; i = i + 1) card_send(got[i]);

        // Our own four went out first, and the monitor read them back off
        // the wire; then the card's echo, which the **host** compares.
        expect_monitor_line(4, {96'd0, 8'hFF, 8'h10, 8'h97, 8'h78});
        expect_card_line(4, {96'd0, 8'hFF, 8'h10, 8'h97, 8'h78});

        host_send(8'h46);                      // 'F'
        expect_banner("+FST");

        card_etu = FAST_DIV;
        repeat (200) @(posedge clk);
        card_send(8'h90);
        card_send(8'h00);
        expect_card_line(2, {112'd0, 8'h90, 8'h00});

        // And one byte out at the fast rate, which is where a terminal that
        // changed its divisor at the wrong moment fails.
        got_n = 0;
        host_send(8'h3A);                      // ':'
        host_send(8'h41); host_send(8'h35);    // A5, one byte needs no pacing
        host_send(8'h0D);                      // CR closes the run
        wait_card_bytes(1);
        if (got[0] !== 8'hA5) begin
            $display("FAIL: the card received %02x at the fast rate, wanted A5",
                     got[0]);
            $finish;
        end
        expect_monitor_line(1, {120'd0, 8'hA5});

        // =============================================================
        // 8. Deactivation, in order
        // =============================================================

        host_send(8'h44);                      // 'D'
        // Reset must drop before power does. Bounded in clocks — powering
        // down settles for 2**VCC_BITS of them, 64 here — and each way of
        // failing says which it was.
        deact = 0;
        while (vcc_en === 1'b1) begin
            @(posedge clk);
            deact = deact + 1;
            if (rst_card) begin
                $display("FAIL: still powered with reset released, %0d clocks after `D`",
                         deact);
                $finish;
            end
            if (deact > 100000) begin
                $display("FAIL: `D` did not power the card down");
                $finish;
            end
        end
        expect_banner("-OFF");

        // ---- A burst across both halves, sent without waiting ----
        //
        // `k s ? k s ? g`, one character time apart: each request arrives
        // while an earlier answer — its own half's or the other's — is still
        // going out. Every one must be answered: two known constants, two
        // card status lines, two display status lines and a dump of PAGES
        // lines, every line whole. On a Basys 3 at 2 Mbaud the second request
        // to a half that was still answering got nothing, because each half
        // kept one flag per request and cleared it before the answer ended.
        // Judged after the fact rather than line by line, because a failure
        // here is an answer that never comes, and waiting for it would hang.
        expect_silence("before the burst");
        host_send(8'h6B);   // k
        host_send(8'h73);   // s
        host_send(8'h3F);   // ?
        host_send(8'h6B);   // k
        host_send(8'h73);   // s
        host_send(8'h3F);   // ?
        host_send(8'h67);   // g
        repeat (HOST_DIV * 10 * 40 * (6 + PAGES)) @(posedge clk);
        begin : burst
            integer lines, known, bad, len, k;
            reg [8*32-1:0] text;
            lines = 0; known = 0; bad = 0; len = 0; text = 0;
            for (k = rx_rd; k < rx_wr; k = k + 1) begin
                if (rxq[k % 4096] == 8'd10) begin
                    if (len != 0) begin
                        lines = lines + 1;
                        if (len != 32) bad = bad + 1;
                        if (len == 32 && text == "0123456789ABCDEFFEDCBA9876543210")
                            known = known + 1;
                    end
                    len = 0; text = 0;
                end else if (rxq[k % 4096] != 8'd13) begin
                    if (!((rxq[k % 4096] >= 8'h30 && rxq[k % 4096] <= 8'h39) ||
                          (rxq[k % 4096] >= 8'h41 && rxq[k % 4096] <= 8'h46)))
                        bad = bad + 1;
                    text = {text[8*31-1:0], rxq[k % 4096]};
                    len = len + 1;
                end
            end
            rx_rd = rx_wr;
            if (lines !== 6 + PAGES || known !== 2 || bad !== 0) begin
                $display("FAIL: a burst of k s ? k s ? g got %0d line(s), %0d of them the known constant and %0d malformed; wanted %0d, 2 and 0",
                         lines, known, bad, 6 + PAGES);
                $finish;
            end
        end

        $display("PASS: one serial port carried both halves — the card activated in order, the real 14-byte ATR received as one line, four bytes sent to the card and read back off the wire by the monitor, a %0dx%0d frame driven bit by bit into the SPI pins and dumped whole while the card was talking into the same port, both status lines answered by their own half and by neither other, both buttons pressed and released, PPS echoed and the rate changed, a byte each way at F/D=%0d, deactivation dropped reset and the clock before power, and a burst of k s ? k s ? g sent without waiting answered in full",
                 COLUMNS, PAGES, FAST_ETU_CYCLES);
        $finish;
    end
endmodule
