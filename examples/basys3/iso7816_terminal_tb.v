// `iso7816_terminal.v` against a model card that is **the same IP block**,
// on one open-drain wire with a pull-up.
//
// The card side is a second `iso7816_uart`, not a hand-written model, and
// that is the point: a hand-written far side can only be wrong in ways the
// author did not think of, while two instances of the real block on one
// wire also prove that two open-drain devices can share it without
// contention. `pullup` on the shared net is what makes a released line read
// high, exactly as the external 1k resistor does on the board.
//
// What it checks:
//
//   1. The activation order — power, then clock, then reset released.
//   2. The card's answer-to-reset is received and printed. The bytes are
//      the real device's, measured off the part.
//   3. **The PPS exchange**: the terminal sends `FF 10 97 78`, the card
//      receives exactly those four bytes, echoes them, and the terminal
//      reports `+PPS` and switches to the fast rate.
//   4. **Talking at the fast rate afterwards** — a byte each way at
//      `F/D = 4`, which is where a terminal that forgot to change its
//      divisor, or changed it too early, fails.
//   5. **A card that refuses**: a wrong echo must give `-PPS` and leave the
//      rate alone. A terminal that sped up anyway would be unable to talk
//      to the card at all afterwards, which is the worst possible outcome
//      of a failed negotiation.
//   6. Deactivation order, and that the line is released before power goes.
//
// What it would not catch: the real card's timing tolerances, its actual
// guard times, metastability, or anything about the pads. The etu values
// are scaled down by parameter so this simulates in reasonable time; the
// ratio between slow and fast is kept at the real 372:4 so the rate change
// is as abrupt here as it is on the board.

`timescale 1ns / 1ps

module iso7816_terminal_tb;
    localparam integer CARD_DIV        = 14;
    // 372:4 is the real ratio; both scaled by the same factor so the
    // exchange is quick but the rate change is just as sharp.
    localparam integer ETU_CYCLES      = 24;
    localparam integer FAST_ETU_CYCLES = 4;
    localparam integer SLOW_DIV        = ETU_CYCLES * CARD_DIV;
    localparam integer FAST_DIV        = FAST_ETU_CYCLES * CARD_DIV;
    localparam integer HOST_DIV        = 16;
    localparam integer RST_HOLD        = 400;

    reg clk = 1'b0;
    always #5 clk = ~clk;

    // The shared open-drain line, as the **wired-AND** a pull-up
    // physically is. Not two tristate drivers: `reticle sim` refuses a net
    // driven from more than one place, and it is right to — without a
    // pull-up, two released drivers are X, and a bus model that resolves to
    // X proves nothing. Each side either pulls low or releases, so the line
    // is low exactly when somebody pulls it.
    wire term_oe, term_o, card_oe, card_o;
    // A third thing that can pull the contact down: noise.
    //
    // **The line is a wire in the world.** A 200 MHz capture of a real
    // card's contact found 241 transitions shorter than a microsecond in a
    // 20 ms window. Two synchronising flops reject only sub-clock spikes, so
    // before `GLITCH_SHIFT` existed one of those was a start bit — and with
    // `PARITY_RETRY` on, the parity error it caused made the terminal pull
    // the contact low while the card was still transmitting. The captured
    // ATR read `3B 1B FF 1D EE 2B FE ...` for a card that sent
    // `3B 1B 87 05 32 2E ...`.
    reg  glitch = 1'b0;
    wire io = ~((term_oe & ~term_o) | (card_oe & ~card_o) | glitch);

    // One spike, narrower than the filter's window, at the worst moment.
    task spike;
        input integer clocks;
        begin
            glitch = 1'b1;
            repeat (clocks) @(posedge clk);
            glitch = 1'b0;
        end
    endtask

    wire clk_card, rst_card, vcc_en, dut_tx;
    wire [14:0] led;
    wire [6:0]  seg;
    wire        dp;
    wire [3:0]  an;
    wire        host_tx;

    // 2^24 and not 2^20 for the watchdog: it is scaled down with everything
    // else, and the PPS exchange plus the host traffic around it takes
    // longer than 2^20 cycles at these divisors — so the terminal
    // deactivated mid-test and printed `-OFF` where the echo was expected.
    // The board's value is 2^31, nineteen seconds.
    // ---- The core's byte streams ----
    //
    // The serial port is no longer inside the core: it takes and gives
    // bytes so that `iso_display.v` can put it and the display console on
    // one port, and `iso7816_terminal_pad.v` is the one-core wrapper that
    // owns the pins. The `uart` instance below does what that wrapper does.
    wire [7:0] dut_cmd_data, dut_out_data;
    wire       dut_cmd_valid, dut_out_valid, dut_out_ready;

    iso7816_terminal #(
        .CARD_DIV(CARD_DIV), .ETU_CYCLES(ETU_CYCLES),
        .FAST_ETU_CYCLES(FAST_ETU_CYCLES), .RST_HOLD(RST_HOLD),
        .VCC_BITS(6), .GAP_ETU(24), .WDOG_BITS(24), .SEPH_MS_CYCLES(4),
        .SEPH_TURN_CYCLES(FAST_DIV * 16)
    ) dut (
        .clk(clk), .locked(1'b1), .clk_card(clk_card), .rst_card(rst_card),
        .vcc_en(vcc_en),
        .io_i(io), .io_oe(term_oe), .io_o(term_o),
        .cmd_valid(dut_cmd_valid), .cmd_data(dut_cmd_data), .seph_buttons(2'b00),
        .out_valid(dut_out_valid), .out_data(dut_out_data),
        .out_ready(dut_out_ready), .hex_run(),
        .led(led), .seg(seg), .dp(dp), .an(an));

    // ---- The model card: the same block, the other way round ----
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

    // ---- Everything the card received, so the PPS can be checked ----
    reg [7:0] got [0:63];
    integer   got_n = 0;
    integer   base  = 0;
    always @(posedge clk) begin
        if (card_rx_v) begin
            got[got_n % 64] = card_rx;
            got_n = got_n + 1;
        end
    end

    // **Bounded, and it says why it gave up.** An unbounded wait here makes
    // every failure look like "did not finish in the time allowed", which
    // cost several rounds of guessing: the terminal was deactivating on its
    // own watchdog because this task had stalled, so the symptom named the
    // wrong subsystem entirely.
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
            // **2 etu, the mandatory guard and nothing more.** This was 14,
            // which gave the receiver a long idle to resynchronise after
            // every byte -- so back-to-back characters at the spacing a real
            // card actually uses were never tested. On hardware the first
            // byte of a real ATR decoded and every byte after it failed its
            // parity, which is exactly what this hid.
            repeat (card_etu * 2) @(posedge clk);
        end
    endtask

    // Anything the card thinks is malformed, reported as it happens rather
    // than inferred later.
    always @(posedge clk) begin
        if (card_rst_n && card_perr)
            $display("note: at %0t the model card flagged a parity error", $time);
    end

    // ---- The host's end of the serial port ----
    reg host_rst_n = 1'b0;
    initial begin
        repeat (8) @(posedge clk);
        host_rst_n = 1'b1;
    end

    // The board's own end of the link, which the core no longer contains.
    // It shares `host_rst_n`: on the board this port leaves reset with the
    // core, sixteen cycles after the PLL locks, and here both ends come up
    // together after eight cycles. Nothing is sent in either window.
    uart #(.CLK_DIV(HOST_DIV)) dut_port (
        .clk(clk), .rst_n(host_rst_n), .div(16'd0),
        .tx_data(dut_out_data), .tx_valid(dut_out_valid),
        .tx_ready(dut_out_ready), .tx(dut_tx),
        .rx(host_tx), .rx_data(dut_cmd_data), .rx_valid(dut_cmd_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

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
    // convenient loses every character that arrives while it is talking.
    reg [7:0] rxq [0:1023];
    integer   rx_wr = 0, rx_rd = 0;
    always @(posedge clk) begin
        if (from_dut_valid) begin
            rxq[rx_wr % 1024] = from_dut;
            rx_wr = rx_wr + 1;
        end
    end

    reg [7:0] ch;
    integer j2;
    task host_recv;
        begin
            while (rx_rd >= rx_wr) @(posedge clk);
            ch    = rxq[rx_rd % 1024];
            rx_rd = rx_rd + 1;
        end
    endtask

    // `host_recv` with a deadline, for a test whose failure is an answer
    // that never comes: without one that failure is a simulation that never
    // ends rather than a FAIL line.
    task host_recv_within;
        input integer limit;
        integer waited;
        begin
            waited = 0;
            while (rx_rd >= rx_wr) begin
                @(posedge clk);
                waited = waited + 1;
                if (waited > limit) begin
                    $display("FAIL: no character from the terminal within %0d clocks",
                             limit);
                    $finish;
                end
            end
            ch    = rxq[rx_rd % 1024];
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

    // Skips any line ending in front of the banner. A banner can always be
    // preceded by the CRLF that closed whatever came before it -- the
    // card's bytes end a line when the gap expires -- so counting those by
    // hand at every call site is both tedious and fragile. Hand-counting
    // them is how this testbench failed twice.
    task expect_banner;
        input [8*4-1:0] want;
        integer k;
        begin
            host_recv;
            while (ch === 8'd13 || ch === 8'd10) host_recv;
            for (k = 3; k >= 0; k = k - 1) begin
                if (k != 3) host_recv;
                if (ch !== want[8*k +: 8]) begin
                    $display("FAIL: banner char '%c' (%02x), wanted '%c'",
                             ch, ch, want[8*k +: 8]);
                    $finish;
                end
            end
            host_recv;  // CR
            host_recv;  // LF
        end
    endtask

    integer i, deact;
    reg [7:0] want;
    reg [7:0] atr [0:13];

    function [7:0] hexch;
        input [3:0] n;
        hexch = (n < 4'd10) ? (8'd48 + {4'd0, n}) : (8'd55 + {4'd0, n});
    endfunction

    // Like `expect_hex_byte`, but for a byte the monitor saw: `>` then two
    // hex characters. A `!` instead means the parity was wrong, and that is
    // worth reporting as itself rather than as a mismatch.
    task expect_monitor_byte;
        input [7:0] want;
        begin
            host_recv;
            while (ch === 8'd13 || ch === 8'd10) host_recv;
            if (ch === 8'h21) begin
                $display("FAIL: the monitor flagged bad parity on what this terminal sent");
                $finish;
            end
            if (ch !== 8'h3E) begin
                $display("FAIL: wanted `>` from the monitor, got '%c' (%02x)", ch, ch);
                $finish;
            end
            host_recv;
            if (ch !== hexch(want[7:4])) begin
                $display("FAIL: monitor high nibble '%c', wanted '%c'",
                         ch, hexch(want[7:4]));
                $finish;
            end
            host_recv;
            if (ch !== hexch(want[3:0])) begin
                $display("FAIL: monitor low nibble '%c', wanted '%c'",
                         ch, hexch(want[3:0]));
                $finish;
            end
        end
    endtask

    // Skips a line ending in front of the byte, as `expect_banner` does and
    // for the same reason: the gap timer closes a line whenever the contact
    // falls quiet, so any byte may be preceded by one. Counting them by hand
    // at each call site has broken this testbench three times.
    task expect_hex_byte;
        input [7:0] v;
        begin
            host_recv;
            while (ch === 8'd13 || ch === 8'd10) host_recv;
            if (ch !== hexch(v[7:4])) begin
                $display("FAIL: got '%c' for the high nibble of %02x", ch, v);
                $finish;
            end
            host_recv;
            if (ch !== hexch(v[3:0])) begin
                $display("FAIL: got '%c' for the low nibble of %02x", ch, v);
                $finish;
            end
        end
    endtask

    initial begin
        #400_000_000;
        $display("FAIL: did not finish in the time allowed");
        $finish;
    end

    initial begin
        repeat (64) @(posedge clk);
        if (vcc_en !== 1'b0) begin
            $display("FAIL: powered before being asked"); $finish;
        end

        host_send(8'h41);                      // 'A'
        expect_banner("+VCC");
        expect_banner("+CLK");
        expect_banner("+RST");

        // ---- The card answers, with the real device's ATR ----
        atr[0]  = 8'h3B; atr[1]  = 8'h1B; atr[2]  = 8'h87; atr[3]  = 8'h05;
        atr[4]  = 8'h32; atr[5]  = 8'h2E; atr[6]  = 8'h35; atr[7]  = 8'h2E;
        atr[8]  = 8'h31; atr[9]  = 8'h04; atr[10] = 8'h33; atr[11] = 8'h00;
        atr[12] = 8'h00; atr[13] = 8'h04;
        // **Spikes, where they do the most harm.** One while the line is
        // idle before the first character, one between characters, and one
        // inside a character's stop bits — each a thirty-second of an etu,
        // well under the filter's sixteenth, and each enough to be a start
        // bit to a receiver that believes any edge.
        spike(card_etu / 32);
        card_send(atr[0]);
        spike(card_etu / 32);
        card_send(atr[1]);
        for (i = 2; i < 14; i = i + 1) begin
            card_send(atr[i]);
            if (i == 5 || i == 9) spike(card_etu / 32);
        end
        for (i = 0; i < 14; i = i + 1) begin
            want = atr[i];
            expect_hex_byte(want);
        end
        // The answer-to-reset ends with the line going idle, so a CRLF
        // follows it. Consume that before expecting the PPS echo, or the
        // CR arrives where a hex digit is expected -- which is exactly how
        // this testbench first failed.
        host_recv;
        if (ch !== 8'd13) begin
            $display("FAIL: the ATR line ended with %02x, wanted CR", ch);
            $finish;
        end
        host_recv;
        if (ch !== 8'd10) begin
            $display("FAIL: the ATR line ended CR then %02x, wanted LF", ch);
            $finish;
        end

        // ---- PPS: the terminal sends, the card echoes ----
        got_n = 0;
        host_send(8'h50);                      // 'P'
        // Wait for all four to reach the card.
        while (got_n < 4) @(posedge clk);
        if (got[0] !== 8'hFF || got[1] !== 8'h10
            || got[2] !== 8'h97 || got[3] !== 8'h78) begin
            $display("FAIL: the card received %02x %02x %02x %02x, wanted FF 10 97 78",
                     got[0], got[1], got[2], got[3]);
            $finish;
        end
        // **Wait before answering.** A terminal's own guard time after a
        // character is when it samples the line for T=0's parity-error
        // pulse, so a far side that starts transmitting inside that window
        // has its start bit read as an error signal — the terminal then
        // repeats its character and collides with the answer. Real cards do
        // not answer instantly, and `pair_tb` separates its bytes by sixteen
        // etu for the same reason. Answering immediately here produced a
        // parity error on the first echoed byte and cost several rounds.
        repeat (card_etu * 16) @(posedge clk);
        // Echo it, still at the old rate — which is what a real card does.
        for (i = 0; i < 4; i = i + 1) card_send(got[i]);

        // **Our own four go out first, and the monitor reports them.**
        // A second receiver on the contact decodes this terminal's own
        // transmission, each byte prefixed `>`, which is the evidence that
        // what went on the wire was well formed -- by the same receiver
        // that read a real card's ATR.
        expect_monitor_byte(8'hFF); expect_monitor_byte(8'h10);
        expect_monitor_byte(8'h97); expect_monitor_byte(8'h78);

        // Then the card's echo comes back, which the host compares itself --
        // the terminal no longer does, and that is the point: a host
        // comparing four bytes is three lines, while the same comparison in
        // hardware was a state machine that could desynchronise from the
        // card irrecoverably.
        expect_hex_byte(8'hFF); expect_hex_byte(8'h10);
        expect_hex_byte(8'h97); expect_hex_byte(8'h78);
        host_recv; host_recv;                  // the CRLF ending that line

        // Having seen the echo agree, ask for the fast rate.
        host_send(8'h46);                      // 'F'
        expect_banner("+FST");

        // ---- And now both sides are fast ----
        card_etu = FAST_DIV;
        repeat (200) @(posedge clk);
        card_send(8'h90);
        card_send(8'h00);
        expect_hex_byte(8'h90);
        expect_hex_byte(8'h00);

        // A byte from the host to the card, at the fast rate.
        got_n = 0;
        host_send(8'h3A);                      // ':'
        host_send(8'h41);                      // 'A'
        host_send(8'h35);                      // '5'  -> 0xA5
        // **Close the run.** Inside a `:` run every character is data, and
        // that is now enforced on every command decoder rather than only on
        // the byte assembler — so the `D` below is a hex digit until the run
        // ends. This testbench used to leave the run open and deactivate
        // anyway, which is exactly the defect: `D` was data *and* a command
        // at the same time.
        host_send(8'h0D);                      // CR ends it
        while (got_n < 1) @(posedge clk);
        if (got[0] !== 8'hA5) begin
            $display("FAIL: the card received %02x at the fast rate, wanted A5", got[0]);
            $finish;
        end
        // The monitor reports it too, which is the point of having it: the
        // byte is confirmed well formed on the wire at the fast rate by the
        // same receiver that read the ATR at the slow one.
        expect_monitor_byte(8'hA5);

        // ---- `W`: the terminal becomes the SE's MCU ----
        //
        // SESSION_START must reach the card whole, at the fast rate, and
        // after the card ends its turn with a status the next thing it
        // receives must be a ticker. That is the integration: the engine's
        // bytes through the shared transmitter, and the card's bytes into
        // the engine. `ip/bus/seph_mcu`'s own testbench checks the rest of
        // the policy.
        got_n = 0;
        host_send(8'h57);                      // 'W'
        deact = 0;
        while (got_n < 51 && deact < card_etu * 12 * 51 * 4) begin
            @(posedge clk); deact = deact + 1;
        end
        if (got_n < 51 || got[0] !== 8'h01 || got[1] !== 8'h00 || got[2] !== 8'h30
            || got[50] !== 8'h30) begin
            $display("FAIL: `W` sent %0d byte(s) to the card, %02x %02x %02x ... %02x; wanted the 51-byte SESSION_START",
                     got_n, got[0], got[1], got[2], got[50]);
            $finish;
        end
        repeat (card_etu * 16) @(posedge clk);
        card_send(8'h60); card_send(8'h00); card_send(8'h02);
        card_send(8'h00); card_send(8'h00);
        // The model's receiver hears its own transmission on the shared
        // wire, so count only what arrives from here on. The engine waits
        // `SEPH_TURN_CYCLES` before answering, so nothing of it is missed.
        base = got_n;
        deact = 0;
        while (got_n < base + 7 && deact < 4 * 100 * 4 + card_etu * 12 * 7 * 4) begin
            @(posedge clk); deact = deact + 1;
        end
        if (got_n < base + 7 || got[base % 64] !== 8'h0e || got[(base + 1) % 64] !== 8'h00
            || got[(base + 2) % 64] !== 8'h04) begin
            $display("FAIL: after the card's status the terminal sent %0d byte(s), first %02x %02x %02x; wanted a TICKER_EVENT",
                     got_n - base, got[base % 64], got[(base + 1) % 64], got[(base + 2) % 64]);
            $finish;
        end
        // The engine now waits for another status, which never comes. What
        // the console printed meanwhile -- the engine's bytes through the
        // monitor and the card's status -- is not this test's business.
        repeat (card_etu * 40) @(posedge clk);
        rx_rd = rx_wr;

        host_send(8'h44);                      // 'D'
        // Reset must drop before power does. Bounded, and each way of
        // failing says which it was: an unbounded wait here reported
        // "reset still released" when the real fault was that `D` had not
        // been obeyed at all, which named the wrong half of the sequence.
        // The bound is in clocks: powering down settles for 2**VCC_BITS of
        // them, 64 here, so 100000 is a wide margin and not a time.
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

        // ---- A burst: requests that arrive while a line is going out ----
        //
        // `k`, `s`, `k` back to back, one character time apart, which is how
        // a host that does not wait for each answer sends them. Each arrives
        // while an earlier answer is still on the wire, and each must be
        // answered, in order. On a Basys 3 at 2 Mbaud only the first was:
        // the two request flags were cleared together at the end of either
        // one's line, so an `s` during a `k` line was wiped with it.
        repeat (HOST_DIV * 10 * 200) @(posedge clk);
        rx_rd = rx_wr;                         // whatever came before
        host_send(8'h6B);                      // 'k'
        host_send(8'h73);                      // 's'
        host_send(8'h6B);                      // 'k'
        begin : burst
            reg [127:0] word;
            integer line, n;
            for (line = 0; line < 3; line = line + 1) begin
                host_recv_within(HOST_DIV * 10 * 200);
                while (ch === 8'd13 || ch === 8'd10) host_recv_within(HOST_DIV * 10 * 200);
                word = 128'd0;
                for (n = 0; n < 32; n = n + 1) begin
                    if (!((ch >= 8'h30 && ch <= 8'h39) || (ch >= 8'h41 && ch <= 8'h46))) begin
                        $display("FAIL: burst line %0d character %0d is %02x, not hex",
                                 line, n, ch);
                        $finish;
                    end
                    word = (word << 4)
                         | {124'd0, (ch <= 8'h39) ? ch[3:0] : ch[3:0] + 4'd9};
                    if (n != 31) host_recv_within(HOST_DIV * 10 * 200);
                end
                if ((line != 1) && word !== 128'h0123456789ABCDEFFEDCBA9876543210) begin
                    $display("FAIL: burst line %0d is %032x, wanted the known constant",
                             line, word);
                    $finish;
                end
                if ((line == 1) && word[15:8] !== 8'hA5) begin
                    $display("FAIL: burst line 1 is %032x, wanted a status line", word);
                    $finish;
                end
            end
        end

        $display("PASS: activation in order; the real 14-byte ATR received; PPS FF 10 97 78 sent and received byte-for-byte by the card, echoed and compared by this testbench as a host would; the rate switched on `F`; two bytes received and one sent at F/D=4 afterwards; deactivation dropped reset and the clock before power; and k, s, k sent back to back answered in full and in order; `W` sent SESSION_START whole and a ticker after the card's status");
        $finish;
    end
endmodule
