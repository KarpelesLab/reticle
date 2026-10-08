// `iso7816_uart` with a **model card** on the other end of the wire.
//
// The card is a far-side model in absolute simulation time, the way
// `ip/bus/spi_display_rx`'s `FarSide` is: every event it causes lands on
// a `$time` ending in 1 or 6, so no pin it drives ever changes in the
// same instant as the clock edge that samples it. That race is one an
// event-driven simulator may resolve either way and a real circuit may
// lose, so the model keeps clear of it by construction rather than by
// hoping. `align` is what holds that invariant, and it is the reason
// every `etu_div` here is even.
//
// **The accounting is checked before the bytes**, and that order is
// deliberate: `ip/bus/spi_display_rx` shipped with two defects that
// masked each other and left one counter permanently one too high, and
// fifteen of its own testbenches missed it because every one of them
// started from the state in which the two faults agreed. So
// `expect_counts` compares all five counters against totals this file
// keeps itself, at every checkpoint, **including before anything is
// driven at all** — and §0 drives the start state that block's §3a says
// has to be driven: the wire held low across reset, at the level
// opposite to the input synchroniser's own reset value.
//
// What each section would catch
//
//   1. A full session at the default rate — 5208 clocks per etu, which is
//      21505 baud with a 112 MHz system clock and an 8 MHz card clock.
//      Catches: a wrong bit period, a wrong bit
//      order, a parity bit of the wrong polarity, a frame of the wrong
//      length, a byte delivered twice or not at all, a guard time shorter
//      than the two etu QUOTED, and `tx_ready` released early. Would not
//      catch: anything about a real card, or about the card clock, which
//      is not in this block.
//
//   2. **A rate change mid-session** — the same wire, 5208 clocks per etu
//      and then 56. Those are the board's own numbers: a 112 MHz system
//      clock, an 8 MHz card clock at divisor 14, and `F/D` going from the
//      default 372 to 4, which is 21505 baud and then 2 Mbaud. This is
//      the requirement the block exists for. It catches a divisor sampled continuously rather than
//      once per character (the character in flight would garble), a
//      divisor latched at reset and never again (the fast characters
//      would be unreadable), and a half-etu derived from the wrong one.
//      The model card changes its own timebase at the same point, so a
//      terminal that ignored the new divisor would mis-sample every bit
//      rather than drift slowly — there is no tolerance in this test.
//
//   3. **Both conventions.** The inverse section captures the nine bits
//      the terminal put on the wire and decodes them **all four ways** —
//      as they are, polarity inverted only, bit order reversed only, and
//      both — and asserts that only the last matches. A block that
//      inverted one of the two and not the other passes a test written
//      any other way; this one it cannot.
//
//   4. **A parity error in each direction.** Card to terminal: the card
//      sends a character with its parity bit deliberately wrong, and the
//      terminal must raise `rx_parity_error`, drive the error pulse at
//      10.5 etu, and the card must see it at 11.5 etu and repeat.
//      Terminal to card: the card **pulls a one of the terminal's own
//      character low** for its whole etu — which is what an open-drain
//      wire makes possible and is a real corruption rather than a
//      pretended one — then signals the error, and the terminal must
//      repeat the character unprompted. Catches a pulse at the wrong
//      time or of the wrong length, a sample at the wrong time, a repeat
//      that never happens, a repeat whose start bit lands inside the far
//      end's own pulse, and a `repeat_count` that disagrees with the
//      pulses driven.
//
//   5. **A card that never answers**: the waiting time expires, exactly
//      one `rx_timeout` strobe is produced per arming, and the block then
//      carries on and sends another character. Catches a hang, a timeout
//      that repeats for ever, and a timeout that never fires. Then a card
//      that **holds the line low**, which is the only section that tells
//      an edge-detected start bit from a level-detected one: the level
//      reading makes a phantom character every eleven etu for as long as
//      the fault lasts and never reports the timeout, because each
//      phantom re-arms the timer.
//
//   5c. **A line held low across reset.** The level opposite to the input
//      synchroniser's own reset value, so each of its registers
//      transitions once as the real level arrives — the same experiment
//      as flipping those reset values. An ungated edge detector decodes
//      that transition as a start bit and nine zeros as a character, and
//      because nine zeros have even parity the only trace is a counter.
//      This is `ip/bus/spi_display_rx/README.md` §3a's fault in this
//      block's shape, and §0 is where it is caught.
//
//   8b. **`active` low, and coming back out of it.** Deasserted in the
//      middle of a transmission: the wire must be released within a
//      clock, `tx_ready` must go low, `tx_abort` must report the
//      character, and nothing may be counted for it. Then the contact is
//      held **low** while the block is inert — what an unpowered card
//      looks like — and `active` is brought back with it still low.
//      Nothing may come of that: a block that reset or froze its input
//      synchroniser while inert would compare an edge against a register
//      that never saw the pin and make a character out of nine zeros.
//
//   8c. **The three polarity parameters**, on a second instance of the
//      block wired to the same contact through an inverting external
//      stage, with the first one held inert so the two never collide.
//      Its enable is active low, its data output is a constant one, and
//      it reads the contact inverted — and it has to exchange a
//      character in each direction through all three of those. An
//      untested parameter is a liability, and these three are exactly
//      the kind nobody notices is wrong until a board is built.
//
//   6. **Contention safety**, as three continuous assertions rather than
//      a moment: `io_o` is never anything but zero, the line is never X
//      or Z, and whenever neither end drives it is high. Plus: the
//      terminal must not assert `io_oe` at all while the card is driving
//      a character, except for the error pulse that is its right.
//
//   7. Every counter against a driven total at nine checkpoints.
//
// What none of it would catch: anything about a real card, a real part, a
// real pad or metastability. The simulator has no metastable resolution,
// so the synchroniser is tested for latency and not for what it is for;
// and `README.md` §6 is the list of everything still unknown.

`timescale 1ns / 1ps

module iso7816_uart_tb;
    // The simulated clock period. Ten nanoseconds rather than the
    // board's 8.93 (112 MHz) because the model card's timebase has to
    // stay on a whole-nanosecond grid, and **the block cannot tell**: it
    // is told clocks per etu and nothing else. So the divisors below are
    // the board's own numbers while the clock here is a round one.
    //
    // A clock edge therefore falls on every multiple of five nanoseconds
    // and nothing the card drives ever does.
    localparam integer CLK_NS = 10;

    // Clocks per etu, as the board will drive them. At 112 MHz an 8 MHz
    // card clock is divisor 14, so `etu_div` is `(F/D) * 14`: the default
    // F=372, D=1 gives 5208, which is 21505 baud, and F/D = 4 gives 56,
    // which is 2 Mbaud. Both even, which is what keeps the card's
    // half-etu off the clock grid — and every card clock that board can
    // make has an even divisor, so every rate does.
    localparam integer SLOW  = 5208;
    localparam integer FAST  = 56;
    // The divisor the sections that are not about a rate run at. A
    // divisor is a number of clocks, so everything above DIV_MIN behaves
    // alike; section 2 is what proves that by running the real numbers.
    localparam integer QUICK = 96;

    reg clk   = 1'b0;
    reg rst_n = 1'b0;

    reg [15:0] etu_div   = 16'd0;
    reg [7:0]  guard_etu = 8'd0;
    reg        conv      = 1'b0;
    reg [23:0] wt_etu    = 24'd0;

    reg [7:0] tx_data  = 8'd0;
    reg       tx_valid = 1'b0;
    reg       rx_ready = 1'b1;
    // The contact is live. The activation sequencer drives this; here it
    // is high for every section but 8b and 8c.
    reg       active   = 1'b1;

    wire tx_ready, tx_abort;
    wire [7:0] rx_data;
    wire rx_valid, rx_parity_error, rx_overrun, rx_timeout;
    wire io_oe, io_o;
    wire [15:0] tx_char_count, rx_char_count, parity_error_count,
                repeat_count, timeout_count;

    // The card's own open-drain driver.
    reg card_oe = 1'b0;

    // A second instance of the block on the **same contact**, with all
    // three polarities inverted and an inverting external stage in front
    // of it: its enable is active low, its data output is a constant one,
    // and the stage pulls the contact down when both of those say so. It
    // reads the contact inverted too. §8c is where it talks; everywhere
    // else it is inert, so the two can share one wire without ever
    // colliding — which is itself what `active` is for.
    reg        inv_active   = 1'b0;
    reg [7:0]  inv_tx_data  = 8'd0;
    reg        inv_tx_valid = 1'b0;
    wire       inv_oe, inv_o, inv_tx_ready, inv_rx_valid, inv_parity_error;
    wire [7:0] inv_rx_data;
    wire [15:0] inv_tx_chars, inv_rx_chars;
    // The external stage: enabled by a low `io_oe`, pulling the contact
    // down when `io_o` is high.
    wire inv_pulls_low = (inv_oe === 1'b0) && (inv_o === 1'b1);

    // **The wire.** Every end pulls low or releases; none can drive high,
    // and the pull-up has it when they all let go. Written as a wired-and
    // rather than as tri-state drivers because that is what the contact
    // is, and because it makes "nothing drives this high" a property of
    // the model as well as of the block.
    wire line = (io_oe | card_oe | inv_pulls_low) ? 1'b0 : 1'b1;

    iso7816_uart #(
        .ETU_DIV(QUICK), .PARITY_RETRY(1), .MAX_REPEAT(3), .COUNT_WIDTH(16)
    ) dut (
        .clk(clk), .rst_n(rst_n), .active(active),
        .etu_div(etu_div), .guard_etu(guard_etu),
        .convention(conv), .wt_etu(wt_etu),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
        .tx_abort(tx_abort),
        .rx_data(rx_data), .rx_ready(rx_ready), .rx_valid(rx_valid),
        .rx_parity_error(rx_parity_error), .rx_overrun(rx_overrun),
        .rx_timeout(rx_timeout),
        .io_i(line), .io_oe(io_oe), .io_o(io_o),
        .tx_char_count(tx_char_count), .rx_char_count(rx_char_count),
        .parity_error_count(parity_error_count),
        .repeat_count(repeat_count), .timeout_count(timeout_count)
    );

    iso7816_uart #(
        .ETU_DIV(QUICK), .PARITY_RETRY(1), .MAX_REPEAT(3), .COUNT_WIDTH(16),
        .OE_INVERT(1), .OUT_INVERT(1), .IN_INVERT(1)
    ) dut_inv (
        .clk(clk), .rst_n(rst_n), .active(inv_active),
        .etu_div(etu_div), .guard_etu(guard_etu),
        .convention(conv), .wt_etu(24'd0),
        .tx_data(inv_tx_data), .tx_valid(inv_tx_valid),
        .tx_ready(inv_tx_ready), .tx_abort(),
        .rx_data(inv_rx_data), .rx_ready(1'b1), .rx_valid(inv_rx_valid),
        .rx_parity_error(inv_parity_error), .rx_overrun(),
        .rx_timeout(),
        .io_i(~line), .io_oe(inv_oe), .io_o(inv_o),
        .tx_char_count(inv_tx_chars), .rx_char_count(inv_rx_chars),
        .parity_error_count(), .repeat_count(), .timeout_count()
    );

    // What the second instance's consumer took, with the same falling-edge
    // sampling the first one's uses.
    reg [7:0] inv_got = 8'd0;
    integer   inv_n_got = 0;
    always @(negedge clk) begin
        if (rst_n && inv_rx_valid) begin
            inv_got  = inv_rx_data;
            inv_n_got = inv_n_got + 1;
        end
    end

    always #5 clk = ~clk;

    // -------------------------------------------------------------------
    // What this file drove, counted here so the block's own counters have
    // something independent to be wrong against
    // -------------------------------------------------------------------
    integer term_sent        = 0;  // bytes handed to the transmitter
    integer card_frames_sent = 0;  // frames the card drove, repeats included
    integer card_bad_sent    = 0;  // ...of those, with the parity wrong
    integer card_pulses      = 0;  // error pulses the card drove at the terminal
    integer card_frames_got  = 0;  // frames the card received
    integer n_timeout        = 0;  // `rx_timeout` strobes seen
    integer n_overrun        = 0;  // `rx_overrun` strobes seen
    integer n_abort          = 0;  // `tx_abort` strobes seen

    // Bytes the consumer took, and the flag each came with.
    reg [7:0] got [0:63];
    reg       got_err [0:63];
    integer   n_got = 0;
    reg       collect = 1'b1;

    // The handshake is sampled on the falling edge, where every output of
    // the block has settled, so nothing here races the edge that produced
    // it. `rx_valid` is high for exactly one period while `rx_ready` is
    // high, so one falling edge sees each character.
    always @(negedge clk) begin
        if (rst_n) begin
            if (collect && rx_valid && rx_ready) begin
                got[n_got]     = rx_data;
                got_err[n_got] = rx_parity_error;
                n_got          = n_got + 1;
            end
            if (rx_timeout) n_timeout = n_timeout + 1;
            if (rx_overrun) n_overrun = n_overrun + 1;
            if (tx_abort)   n_abort   = n_abort + 1;
        end
    end

    // -------------------------------------------------------------------
    // The three continuous assertions of §6
    // -------------------------------------------------------------------

    // Set while the card is driving a character, so that the terminal
    // asserting `io_oe` then is a fault. The terminal's own error pulse
    // comes after the card has released, so there is no legitimate
    // overlap to make an exception for.
    reg card_driving = 1'b0;

    // Only once reset has been released: this block has no initial
    // values and its registers hold X until then, which is deliberate —
    // `ip/bus/uart` is the same and `CLAUDE.md` records what tying a
    // reset to its inactive level has cost here.
    always @(negedge clk) if (rst_n) begin
        if (io_o !== 1'b0) begin
            $display("FAIL: io_o is %b at %0t; nothing may drive this line high",
                     io_o, $time);
            $finish;
        end
        if (line !== 1'b0 && line !== 1'b1) begin
            $display("FAIL: the line is %b at %0t", line, $time);
            $finish;
        end
        if (io_oe === 1'b0 && card_oe === 1'b0 && !inv_pulls_low
            && line !== 1'b1) begin
            $display("FAIL: every end released the line and it reads %b at %0t",
                     line, $time);
            $finish;
        end
        // The inverted instance's data output is a constant too, just a
        // different one: it pulls the contact down through its external
        // stage or lets go, and never drives both.
        if (inv_o !== 1'b1) begin
            $display("FAIL: inv_o is %b at %0t; it is a constant one", inv_o, $time);
            $finish;
        end
        if (card_driving && io_oe !== 1'b0) begin
            $display("FAIL: the terminal drove the wire at %0t while the card had it",
                     $time);
            $finish;
        end
    end

    // -------------------------------------------------------------------
    // The card's timebase
    // -------------------------------------------------------------------
    integer etu_ns  = QUICK * CLK_NS;
    integer half_ns = (QUICK * CLK_NS) / 2;

    task set_rate;
        input integer d;
        begin
            if ((d % 2) != 0) begin
                $display("FAIL: this testbench needs an even etu_div, not %0d", d);
                $finish;
            end
            etu_div = d[15:0];
            etu_ns  = d * CLK_NS;
            half_ns = (d * CLK_NS) / 2;
        end
    endtask

    // Moves to the next `$time` congruent to 1 modulo the clock period,
    // after which every delay of a whole or a half etu keeps the card off
    // the clock grid — because an even divisor makes both multiples of
    // ten.
    task align;
        begin
            while (($time % CLK_NS) != 1) #1;
        end
    endtask

    task wait_etu;
        input integer n;
        begin
            #(n * etu_ns);
        end
    endtask

    // -------------------------------------------------------------------
    // Frames, as nine electrical bits with the first at index zero
    // -------------------------------------------------------------------
    reg [8:0] nine;

    function [7:0] rev8;
        input [7:0] v;
        begin
            rev8 = {v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]};
        end
    endfunction

    // The four readings of one captured frame. Only `dec_both` is the
    // inverse convention; the other three are what a block that got half
    // of it right would produce.
    function [7:0] dec_plain;   input [8:0] n; begin dec_plain   =  n[7:0];        end endfunction
    function [7:0] dec_pol;     input [8:0] n; begin dec_pol     = ~n[7:0];        end endfunction
    function [7:0] dec_order;   input [8:0] n; begin dec_order   =  rev8(n[7:0]);  end endfunction
    function [7:0] dec_both;    input [8:0] n; begin dec_both    = ~rev8(n[7:0]);  end endfunction

    // -------------------------------------------------------------------
    // The terminal's producer side
    // -------------------------------------------------------------------

    // Hands one byte to the transmitter and returns at the instant its
    // start bit falls, so a caller can walk the frame from there.
    task term_post;
        input [7:0] b;
        begin
            @(negedge clk);
            while (tx_ready !== 1'b1) @(negedge clk);
            tx_data   = b;
            tx_valid  = 1'b1;
            term_sent = term_sent + 1;
            // The next rising edge accepts it and the start bit falls in
            // that same instant.
            @(negedge line);
        end
    endtask

    // -------------------------------------------------------------------
    // The card receiving one of the terminal's characters
    //
    // Called at the instant the start bit falls. `glitch0` pulls the
    // **first data bit** low for its whole etu, which an open-drain wire
    // allows whenever the terminal was releasing it for a one — a real
    // corruption of a real character, rather than a card that pretends to
    // have seen one.
    // -------------------------------------------------------------------
    task card_watch;
        input integer glitch0;
        integer i;
        begin
            // Let go of `tx_valid` one clock in, so a second byte is not
            // accepted. The transmitter is busy for the rest of the
            // frame, so this cannot be seen.
            @(negedge clk);
            tx_valid     = 1'b0;
            inv_tx_valid = 1'b0;
            // To the middle of the start bit, one nanosecond off the
            // clock grid. Half an etu less the clock the line above cost.
            #(half_ns - (CLK_NS / 2) + 1);
            if (line !== 1'b0) begin
                $display("FAIL: no start bit at %0t: the line reads %b", $time, line);
                $finish;
            end
            if (glitch0 != 0) begin
                #(half_ns); card_oe = 1'b1;  // 1.0 etu: take the bit low
                #(half_ns); nine[0] = line;  // 1.5 etu
                #(half_ns); card_oe = 1'b0;  // 2.0 etu: release it again
                #(half_ns); nine[1] = line;  // 2.5 etu
                for (i = 2; i < 9; i = i + 1) begin
                    #(etu_ns);
                    nine[i] = line;
                end
            end else begin
                for (i = 0; i < 9; i = i + 1) begin
                    #(etu_ns);
                    nine[i] = line;
                end
            end
            card_frames_got = card_frames_got + 1;
            // 9.5 etu: the parity bit is in.
        end
    endtask

    // Waits for a start edge and then does the same, for a character the
    // card did not know was coming — a repeat, which is the one place the
    // terminal's own choice of delay must not be written into this file.
    task card_watch_react;
        begin
            @(negedge line);
            card_watch(0);
        end
    endtask

    // The parity of a captured frame, as each convention reads it: even
    // over nine logic ones is odd over the nine on the wire once the
    // inverse convention has complemented all nine.
    function parity_ok;
        input [8:0] n;
        input       c;
        begin
            parity_ok = (^n) === c;
        end
    endfunction

    // The error pulse, from 10.5 to 12.5 etu. Called at 9.5 etu.
    task card_pulse;
        begin
            #(etu_ns);            // 10.5 etu
            card_oe = 1'b1;
            #(2 * etu_ns);        // 12.5 etu
            card_oe = 1'b0;
            card_pulses = card_pulses + 1;
        end
    endtask

    // -------------------------------------------------------------------
    // The card sending one character
    // -------------------------------------------------------------------
    reg card_saw_pulse = 1'b0;

    task card_send;
        input [7:0] b;
        input       bad;   // corrupt the parity bit
        integer i;
        reg [8:0] f;
        reg p;
        begin
            align;
            p = ^b;
            if (bad) p = ~p;
            // Direct: the data least significant first then the parity
            // bit, at the level each logic value means. Inverse:
            // reversed and complemented, parity bit included.
            if (conv === 1'b0) f = {p, b};
            else               f = ~{p, rev8(b)};
            card_driving     = 1'b1;
            card_oe          = 1'b1;          // the start bit
            #(etu_ns);
            for (i = 0; i < 9; i = i + 1) begin
                card_oe = ~f[i];              // pull low for an electrical zero
                #(etu_ns);
            end
            card_oe          = 1'b0;          // released: the guard time
            card_driving     = 1'b0;
            card_frames_sent = card_frames_sent + 1;
            if (bad) card_bad_sent = card_bad_sent + 1;
            // 10.0 etu. The terminal's error pulse, if it saw bad parity,
            // runs from 10.5 to 12.5; sample it at 11.5 as QUOTED.
            #(etu_ns + half_ns);
            card_saw_pulse = (line === 1'b0);
            #(half_ns);                       // 12.0 etu: the guard is over
            if (card_saw_pulse) begin
                // Stay clear of the terminal's own pulse before repeating.
                #(2 * etu_ns);                // 14.0 etu
            end
        end
    endtask

    // -------------------------------------------------------------------
    // The accounting
    // -------------------------------------------------------------------
    task expect_counts;
        input integer where;
        begin
            // A character in flight has not been counted yet, and its
            // guard time is part of it: `tx_ready` coming back is the
            // block's own statement that nothing is outstanding. Then
            // four clocks, because each counter moves one cycle after
            // the event it counts.
            repeat (4) @(negedge clk);
            while (tx_ready !== 1'b1) @(negedge clk);
            repeat (4) @(negedge clk);
            if (tx_char_count !== term_sent) begin
                $display("FAIL(%0d): tx_char_count %0d, drove %0d",
                         where, tx_char_count, term_sent);
                $finish;
            end
            if (rx_char_count !== card_frames_sent) begin
                $display("FAIL(%0d): rx_char_count %0d, the card drove %0d frames",
                         where, rx_char_count, card_frames_sent);
                $finish;
            end
            if (parity_error_count !== card_bad_sent) begin
                $display("FAIL(%0d): parity_error_count %0d, the card corrupted %0d",
                         where, parity_error_count, card_bad_sent);
                $finish;
            end
            if (repeat_count !== card_pulses) begin
                $display("FAIL(%0d): repeat_count %0d, the card drove %0d error pulses",
                         where, repeat_count, card_pulses);
                $finish;
            end
            if (timeout_count !== n_timeout) begin
                $display("FAIL(%0d): timeout_count %0d, but %0d strobes were seen",
                         where, timeout_count, n_timeout);
                $finish;
            end
            if (n_got !== card_frames_sent && collect) begin
                $display("FAIL(%0d): %0d bytes reached the consumer, the card sent %0d",
                         where, n_got, card_frames_sent);
                $finish;
            end
        end
    endtask

    task expect_byte;
        input integer where;
        input integer idx;
        input [7:0]   want;
        input         want_err;
        begin
            if (got[idx] !== want) begin
                $display("FAIL(%0d): byte %0d is %02x, wanted %02x",
                         where, idx, got[idx], want);
                $finish;
            end
            if (got_err[idx] !== want_err) begin
                $display("FAIL(%0d): byte %0d has rx_parity_error %b, wanted %b",
                         where, idx, got_err[idx], want_err);
                $finish;
            end
        end
    endtask

    // The byte that arrived most recently, which is what a card_send is
    // checked against. Index-free on purpose: a check written against an
    // absolute position has to be renumbered whenever a section is added,
    // and a renumbering is a way to make a passing test meaningless.
    task expect_last;
        input integer where;
        input [7:0]   want;
        input         want_err;
        begin
            repeat (4) @(negedge clk);
            if (n_got == 0) begin
                $display("FAIL(%0d): no byte reached the consumer", where);
                $finish;
            end
            expect_byte(where, n_got - 1, want, want_err);
        end
    endtask

    // One of the terminal's characters, received and checked by the card
    // in whichever convention is in force.
    task card_expect;
        input integer where;
        input [7:0]   want;
        begin
            if (!parity_ok(nine, conv)) begin
                $display("FAIL(%0d): the terminal's parity is wrong: nine bits %b, convention %b",
                         where, nine, conv);
                $finish;
            end
            if (conv === 1'b0) begin
                if (dec_plain(nine) !== want) begin
                    $display("FAIL(%0d): the terminal sent %02x, wanted %02x (direct); nine bits %b",
                             where, dec_plain(nine), want, nine);
                    $finish;
                end
            end else begin
                if (dec_both(nine) !== want) begin
                    $display("FAIL(%0d): the terminal sent %02x, wanted %02x (inverse); nine bits %b",
                             where, dec_both(nine), want, nine);
                    $finish;
                end
            end
        end
    endtask

    integer i;
    reg [8:0] captured;

    // The watchdog. Everything below is about three milliseconds of
    // simulated time, most of it the two characters at 5208 clocks an
    // etu, so twenty is room and not a tolerance.
    initial begin
        #20_000_000;
        $display("FAIL: did not finish in the time allowed");
        $finish;
    end

    initial begin
        set_rate(QUICK);
        guard_etu = 8'd0;
        conv      = 1'b0;
        wt_etu    = 24'd0;

        // **The line is held low across reset**, which is the state a
        // dead card, a shorted contact or a far end that has the wire
        // puts a terminal in. It is also the level *opposite* to the one
        // the input synchroniser resets to, so every register in that
        // chain transitions once as the real level propagates — which is
        // the same experiment as flipping their reset values, and the
        // experiment `ip/bus/spi_display_rx`'s §3a says has to be run.
        //
        // Nothing may come of it: the start bit is an edge, and an edge
        // compared against a reset value is not an observation of a pin.
        // An ungated detector makes a start bit here and decodes nine
        // zeros as a character, whose parity is legitimately even, so the
        // only trace of it is a counter.
        card_oe = 1'b1;
        repeat (4) @(negedge clk);
        rst_n = 1'b1;
        // Held low for longer than a whole character, because the
        // manufactured start bit is only rejected as a glitch if the line
        // comes back **before** the middle of it: releasing the wire too
        // early is what makes this test pass against the fault.
        wait_etu(12);
        if (rx_char_count !== 0 || rx_valid !== 1'b0) begin
            $display("FAIL: a line held low across reset made %0d character(s); a synchroniser's reset value is not an edge",
                     rx_char_count);
            $finish;
        end
        card_oe = 1'b0;
        repeat (8) @(negedge clk);

        // =============================================================
        // 0. Nothing driven yet
        //
        // The case `spi_display_rx` failed and nothing checked. Every
        // counter must be zero, and the block must have let go of the
        // wire rather than come up holding it.
        // =============================================================
        expect_counts(`__LINE__);
        if (tx_char_count !== 0 || rx_char_count !== 0
            || parity_error_count !== 0 || repeat_count !== 0
            || timeout_count !== 0) begin
            $display("FAIL: a counter moved before anything was driven");
            $finish;
        end
        if (io_oe !== 1'b0 || io_o !== 1'b0 || line !== 1'b1) begin
            $display("FAIL: after reset io_oe=%b io_o=%b line=%b; the wire must idle high",
                     io_oe, io_o, line);
            $finish;
        end
        if (tx_ready !== 1'b1) begin
            $display("FAIL: tx_ready is %b after reset", tx_ready); $finish;
        end
        if (rx_valid !== 1'b0 || rx_parity_error !== 1'b0
            || rx_overrun !== 1'b0 || rx_timeout !== 1'b0
            || tx_abort !== 1'b0) begin
            $display("FAIL: an output came up asserted after reset");
            $finish;
        end

        // =============================================================
        // 1. A full session at the default rate, direct convention
        //
        // 5208 clocks an etu is 21505 baud on the board. The terminal sends
        // a two-byte command; the card answers with a status word.
        // =============================================================
        set_rate(SLOW);
        wait_etu(2);

        term_post(8'h3B); card_watch(0); card_expect(`__LINE__, 8'h3B);
        // The guard time: the character ended at 10.0 etu and the
        // transmitter must not be ready again before 12.0.
        #(2 * etu_ns);                        // 11.5 etu
        if (tx_ready !== 1'b0) begin
            $display("FAIL: tx_ready came back at 11.5 etu, before the 2 etu guard");
            $finish;
        end
        #(etu_ns);                            // 12.5 etu
        if (tx_ready !== 1'b1) begin
            $display("FAIL: tx_ready is still low at 12.5 etu with guard_etu 0");
            $finish;
        end

        term_post(8'hA4); card_watch(0); card_expect(`__LINE__, 8'hA4);
        expect_counts(`__LINE__);

        // The card answers. Three etu clear of the terminal's guard.
        wait_etu(3);
        card_send(8'h90, 1'b0);
        expect_last(`__LINE__, 8'h90, 1'b0);
        wait_etu(1);
        card_send(8'h00, 1'b0);
        expect_last(`__LINE__, 8'h00, 1'b0);
        expect_counts(`__LINE__);
        if (n_got !== 2) begin
            $display("FAIL: %0d bytes arrived at the default rate, wanted 2", n_got);
            $finish;
        end

        // =============================================================
        // 2. A rate change mid-session: 21.5 kbaud to 2 Mbaud
        //
        // The wire, the block and the model card all carry on; only
        // `etu_div` moves, and it moves between characters. This is the
        // requirement.
        // =============================================================
        wait_etu(3);
        set_rate(FAST);
        // One etu at the **new** rate of settling, so the change lands
        // while the line is idle, which is the only time it is defined.
        wait_etu(2);

        term_post(8'h5A); card_watch(0); card_expect(`__LINE__, 8'h5A);
        term_post(8'hC3); card_watch(0); card_expect(`__LINE__, 8'hC3);
        expect_counts(`__LINE__);

        wait_etu(3);
        card_send(8'h6F, 1'b0);
        expect_last(`__LINE__, 8'h6F, 1'b0);
        wait_etu(1);
        card_send(8'h00, 1'b0);
        expect_last(`__LINE__, 8'h00, 1'b0);
        expect_counts(`__LINE__);
        if (n_got !== 4) begin
            $display("FAIL: %0d bytes after the rate change, wanted 4", n_got);
            $finish;
        end

        // And back down again, because a PPS that failed leaves the link
        // where it started and the block must be able to go either way.
        wait_etu(3);
        set_rate(QUICK);
        wait_etu(2);
        term_post(8'h21); card_watch(0); card_expect(`__LINE__, 8'h21);
        wait_etu(3);
        card_send(8'h12, 1'b0);
        expect_last(`__LINE__, 8'h12, 1'b0);
        expect_counts(`__LINE__);

        // And a rate the terminal has **not transmitted at**. The card
        // speaks first after this change, so the only thing that can have
        // taken the new divisor is the latch the *receiver* makes at the
        // start edge: a receiver that reused whatever the last
        // transmission left behind would sample this character at half
        // the rate and read rubbish.
        wait_etu(3);
        set_rate(QUICK * 2);
        wait_etu(2);
        card_send(8'h4D, 1'b0);
        expect_last(`__LINE__, 8'h4D, 1'b0);
        expect_counts(`__LINE__);
        set_rate(QUICK);
        wait_etu(2);

        // =============================================================
        // 3. The inverse convention, and that it inverts *both* things
        //
        // 0x3B's bit reversal is 0xDC, its complement 0xC4 and its
        // reversed complement 0x23: four different bytes, so no two of
        // the four readings below can be confused.
        // =============================================================
        wait_etu(3);
        conv = 1'b1;
        wait_etu(2);

        term_post(8'h3B); card_watch(0);
        captured = nine;
        card_expect(`__LINE__, 8'h3B);
        // The four-way decode. Only the full inverse reading may match.
        if (dec_both(captured) !== 8'h3B) begin
            $display("FAIL: the inverse reading of %b is %02x, wanted 3b",
                     captured, dec_both(captured));
            $finish;
        end
        if (dec_plain(captured) === 8'h3B) begin
            $display("FAIL: the frame decodes as 3b with neither inversion, so the terminal sent a direct frame");
            $finish;
        end
        if (dec_pol(captured) === 8'h3B) begin
            $display("FAIL: the frame decodes as 3b with the polarity inverted alone, so the bit order was not reversed");
            $finish;
        end
        if (dec_order(captured) === 8'h3B) begin
            $display("FAIL: the frame decodes as 3b with the order reversed alone, so the polarity was not inverted");
            $finish;
        end
        // The parity bit of an inverse frame is **odd** over the nine
        // bits on the wire, which is the other half of the same fact.
        if ((^captured) !== 1'b1) begin
            $display("FAIL: an inverse frame's nine wire bits have even parity: %b", captured);
            $finish;
        end

        // A second byte, chosen because its reversal is itself: 0x81
        // reverses to 0x81, so this one is caught only by the polarity.
        // The pair together is what makes the section whole.
        term_post(8'h81); card_watch(0); card_expect(`__LINE__, 8'h81);
        expect_counts(`__LINE__);

        // And the card answers in the inverse convention, so the receive
        // path is held to the same rule as the transmit path.
        wait_etu(3);
        card_send(8'h3B, 1'b0);
        expect_last(`__LINE__, 8'h3B, 1'b0);
        wait_etu(1);
        card_send(8'h81, 1'b0);
        expect_last(`__LINE__, 8'h81, 1'b0);
        expect_counts(`__LINE__);

        wait_etu(3);
        conv = 1'b0;
        wait_etu(2);

        // =============================================================
        // 4a. A parity error the card made: the terminal pulses and the
        //     card repeats
        // =============================================================
        card_send(8'h7E, 1'b1);               // parity bit deliberately wrong
        if (card_saw_pulse !== 1'b1) begin
            $display("FAIL: the terminal did not drive an error pulse for a bad character");
            $finish;
        end
        // The bad character was still delivered, with its flag, which is
        // what `uart_frame_rx` does and for the same reason.
        expect_last(`__LINE__, 8'h7E, 1'b1);
        // `card_send` has already waited clear of the pulse. Repeat it,
        // correctly this time.
        card_send(8'h7E, 1'b0);
        if (card_saw_pulse !== 1'b0) begin
            $display("FAIL: the terminal pulsed at a character whose parity was right");
            $finish;
        end
        expect_last(`__LINE__, 8'h7E, 1'b0);
        expect_counts(`__LINE__);
        if (parity_error_count !== 1) begin
            $display("FAIL: parity_error_count is %0d, wanted 1", parity_error_count);
            $finish;
        end

        // =============================================================
        // 4b. A parity error the card *caused*, in the terminal's own
        //     character, by holding one of its ones low
        //
        // The terminal must repeat the character of its own accord when
        // it finds the line low at 11.5 etu, and the repeat must arrive
        // intact. `repeat_count` must agree with the pulses driven.
        // =============================================================
        wait_etu(3);
        term_post(8'h3B);
        card_watch(1);                        // the card takes bit 0 low
        if (parity_ok(nine, conv)) begin
            $display("FAIL: pulling a one low left the parity correct: %b", nine);
            $finish;
        end
        card_pulse;                           // 10.5 to 12.5 etu
        // The repeat: the card does not know when it will come, so it
        // waits for the edge rather than for a time this file chose.
        card_watch_react;
        card_expect(`__LINE__, 8'h3B);
        expect_counts(`__LINE__);
        if (repeat_count !== 1) begin
            $display("FAIL: repeat_count is %0d after one error pulse, wanted 1",
                     repeat_count);
            $finish;
        end
        if (n_abort !== 0) begin
            $display("FAIL: %0d aborts, but the repeat succeeded", n_abort);
            $finish;
        end

        // =============================================================
        // 5. A card that never answers
        //
        // The waiting time expires once per arming and the block carries
        // on. Twenty etu is a waiting time; a real one is hundreds.
        // =============================================================
        wait_etu(3);
        wt_etu = 24'd20;
        wait_etu(1);
        term_post(8'h00); card_watch(0); card_expect(`__LINE__, 8'h00);
        // The guard runs to 12 etu and the timer then has twenty more.
        wait_etu(14);
        if (n_timeout !== 0) begin
            $display("FAIL: the waiting time expired early: %0d strobe(s) at 14 etu",
                     n_timeout);
            $finish;
        end
        wait_etu(20);
        if (n_timeout !== 1) begin
            $display("FAIL: %0d timeout strobes after the waiting time, wanted 1",
                     n_timeout);
            $finish;
        end
        // ...and exactly once, however long nothing happens.
        wait_etu(60);
        if (n_timeout !== 1) begin
            $display("FAIL: %0d timeout strobes; a timeout must not repeat per arming",
                     n_timeout);
            $finish;
        end
        expect_counts(`__LINE__);
        if (timeout_count !== 1) begin
            $display("FAIL: timeout_count is %0d, wanted 1", timeout_count);
            $finish;
        end
        // Not a hang: the link still works.
        wt_etu = 24'd0;
        wait_etu(1);
        term_post(8'hFF); card_watch(0); card_expect(`__LINE__, 8'hFF);
        wait_etu(3);
        card_send(8'hAA, 1'b0);
        expect_last(`__LINE__, 8'hAA, 1'b0);
        expect_counts(`__LINE__);

        // =============================================================
        // 5b. A line held low — a dead or a shorted card
        //
        // **The start bit is an edge and not a level.** A block that
        // watched the level would make a character out of a low line,
        // then another, for as long as it stayed low, and would never
        // report the timeout, because every phantom character re-arms
        // the timer. Watching the edge makes exactly one character — the
        // transition into the fault, nine zeros, whose parity is
        // legitimately even — and then the waiting time expires and says
        // so.
        //
        // This is the only section that tells the two apart: every other
        // one happens to let the level reading recover, because a
        // spurious start is rejected half an etu later when the line
        // reads high.
        // =============================================================
        wait_etu(3);
        wt_etu = 24'd20;
        wait_etu(2);
        align;
        card_oe = 1'b1;                       // ...and never let go
        wait_etu(60);
        card_oe = 1'b0;
        // The one character the edge into the fault produced.
        card_frames_sent = card_frames_sent + 1;
        repeat (4) @(negedge clk);
        if (rx_char_count !== card_frames_sent) begin
            $display("FAIL: a line held low for 60 etu produced %0d characters, wanted %0d; the start bit must be an edge",
                     rx_char_count, card_frames_sent);
            $finish;
        end
        if (n_timeout !== 2) begin
            $display("FAIL: %0d timeout strobes with the line held low, wanted 2", n_timeout);
            $finish;
        end
        expect_last(`__LINE__, 8'h00, 1'b0);
        wt_etu = 24'd0;
        expect_counts(`__LINE__);

        // =============================================================
        // 6. An overrun, which is what the receive handshake is for
        //
        // With `rx_ready` low the character is held; a second one over it
        // raises `rx_overrun` once and the newer character wins.
        // =============================================================
        wait_etu(3);
        collect  = 1'b0;
        rx_ready = 1'b0;
        card_send(8'h11, 1'b0);
        wait_etu(1);
        card_send(8'h22, 1'b0);
        repeat (4) @(negedge clk);
        if (n_overrun !== 1) begin
            $display("FAIL: %0d overruns for two characters nobody took, wanted 1",
                     n_overrun);
            $finish;
        end
        if (rx_data !== 8'h22) begin
            $display("FAIL: after an overrun rx_data is %02x; the newer character must win",
                     rx_data);
            $finish;
        end
        if (rx_valid !== 1'b1) begin
            $display("FAIL: rx_valid dropped with nobody having taken the character");
            $finish;
        end
        rx_ready = 1'b1;
        repeat (4) @(negedge clk);
        if (rx_valid !== 1'b0) begin
            $display("FAIL: rx_valid did not clear when the character was taken");
            $finish;
        end
        collect = 1'b1;
        // Two frames arrived and two were counted, even though one byte
        // was lost: `rx_char_count` counts characters on the wire.
        if (rx_char_count !== card_frames_sent) begin
            $display("FAIL: rx_char_count %0d, the card drove %0d frames",
                     rx_char_count, card_frames_sent);
            $finish;
        end
        // The consumer took one of the two, so the byte tally is behind
        // by the one the overrun destroyed. Bring it level.
        n_got = card_frames_sent;

        // =============================================================
        // 7. A guard time longer than the minimum, as a run-time input
        // =============================================================
        wait_etu(3);
        guard_etu = 8'd4;
        wait_etu(1);
        term_post(8'h55); card_watch(0); card_expect(`__LINE__, 8'h55);
        #(2 * etu_ns);                        // 11.5 etu
        if (tx_ready !== 1'b0) begin
            $display("FAIL: tx_ready at 11.5 etu with guard_etu 4"); $finish;
        end
        wait_etu(4);                          // 15.5 etu
        if (tx_ready !== 1'b0) begin
            $display("FAIL: tx_ready at 15.5 etu with guard_etu 4, wanted 16");
            $finish;
        end
        wait_etu(1);                          // 16.5 etu
        if (tx_ready !== 1'b1) begin
            $display("FAIL: tx_ready still low at 16.5 etu with guard_etu 4");
            $finish;
        end
        guard_etu = 8'd0;
        expect_counts(`__LINE__);

        // =============================================================
        // 8. A divisor below DIV_MIN falls back to the parameter
        //
        // `uart_rx`'s rule: a rate that cannot be expressed leaves the
        // port working rather than stopping the block. ETU_DIV is QUICK
        // here, so the link keeps running at QUICK when `etu_div` is 1.
        // =============================================================
        wait_etu(3);
        etu_div = 16'd1;
        wait_etu(2);
        term_post(8'h0F); card_watch(0); card_expect(`__LINE__, 8'h0F);
        wait_etu(3);
        card_send(8'hF0, 1'b0);
        // The fallback is what makes this readable at all: at an etu_div
        // of one the card's frame would be unsamplable.
        expect_last(`__LINE__, 8'hF0, 1'b0);
        expect_counts(`__LINE__);

        // =============================================================
        // 8b. `active` low: inert, and coming back out of it
        //
        // The hazard requirement is that the wire is released before the
        // card's VCC drops, **whatever else is going on**. So this
        // deasserts `active` three etu into a transmission.
        // =============================================================
        wait_etu(3);
        set_rate(QUICK);
        wait_etu(2);

        term_post(8'h99);
        @(negedge clk);
        tx_valid = 1'b0;
        // Three bits into the frame, which for 0x99 is a data bit the
        // transmitter is **driving** — so there is something to release.
        wait_etu(3);
        @(negedge clk);
        if (io_oe !== 1'b1) begin
            $display("FAIL: the transmitter is not driving three bits into 0x99's frame, so this section proves nothing");
            $finish;
        end
        active = 1'b0;
        // One rising edge, and nothing more. Checking this four clocks
        // later would prove nothing: forcing `state` back to idle
        // releases the wire a clock later anyway, so only the first
        // clock tells the release apart from the state machine's own
        // recovery — and the hazard is about the first clock.
        @(negedge clk);
        if (io_oe !== 1'b0) begin
            $display("FAIL: io_oe is %b on the clock after `active` fell; the wire must be released before VCC drops",
                     io_oe);
            $finish;
        end
        repeat (3) @(negedge clk);
        if (line !== 1'b1) begin
            $display("FAIL: the released contact reads %b", line); $finish;
        end
        if (tx_ready !== 1'b0) begin
            $display("FAIL: tx_ready is high while the block is inert"); $finish;
        end
        if (n_abort !== 1) begin
            $display("FAIL: %0d aborts after a transmission was abandoned, wanted 1",
                     n_abort);
            $finish;
        end
        // The character never left, so it is not a character this block
        // sent. `tx_char_count` counts transmissions that completed.
        term_sent = term_sent - 1;

        // And the other hazard: the contact held low while inert, with
        // `active` brought back on top of it. An unpowered card looks
        // exactly like this, and nothing may come of it.
        align;
        card_oe = 1'b1;
        wait_etu(2);
        active = 1'b1;
        wait_etu(12);
        if (rx_char_count !== card_frames_sent) begin
            $display("FAIL: coming out of the inert state on a low contact made a character: rx_char_count %0d, the card drove %0d",
                     rx_char_count, card_frames_sent);
            $finish;
        end
        card_oe = 1'b0;
        wait_etu(2);

        // And the sharper version of the same hazard: the contact falls
        // and `active` comes back **in the same instant**, so the edge is
        // still inside the comparison window when the block wakes. The
        // synchroniser having kept running is not enough here — both
        // halves of the comparison are real observations, and one of them
        // is the level from before the contact was live. What makes it
        // safe is that the settle window is **reopened** when `active`
        // rises, so no edge is taken until both halves were sampled while
        // active. A `settle_sr` that was only counted out once, at reset,
        // takes this one.
        active  = 1'b0;
        card_oe = 1'b0;
        repeat (8) @(negedge clk);            // the chain reads a released line
        align;
        card_oe = 1'b1;
        active  = 1'b1;
        wait_etu(12);
        if (rx_char_count !== card_frames_sent) begin
            $display("FAIL: an edge from before `active` rose made a character: rx_char_count %0d, the card drove %0d",
                     rx_char_count, card_frames_sent);
            $finish;
        end
        card_oe = 1'b0;
        wait_etu(2);

        // Not a hang: the link still works.
        term_post(8'h99); card_watch(0); card_expect(`__LINE__, 8'h99);
        wait_etu(3);
        card_send(8'h66, 1'b0);
        expect_last(`__LINE__, 8'h66, 1'b0);
        expect_counts(`__LINE__);

        // =============================================================
        // 8c. The three polarity parameters
        //
        // The same block again on the same contact, with `OE_INVERT`,
        // `OUT_INVERT` and `IN_INVERT` all set and an inverting external
        // stage in front of it. The first instance is held inert so the
        // two never collide — which is what `active` is for — and the
        // second has to exchange a character in each direction through
        // all three inversions.
        // =============================================================
        wait_etu(3);
        active     = 1'b0;
        repeat (4) @(negedge clk);
        inv_active = 1'b1;
        wait_etu(3);

        // The card speaks first, so the inverted receive path is what
        // reads it: `IN_INVERT` and nothing else.
        card_send(8'h4B, 1'b0);
        // That frame went to the inverted instance. The first one was
        // inert and did not take it, so it is not a frame this file drove
        // at *it* and the running tally must not count it.
        card_frames_sent = card_frames_sent - 1;
        repeat (4) @(negedge clk);
        if (inv_n_got !== 1 || inv_got !== 8'h4B) begin
            $display("FAIL: the inverted instance read %0d byte(s), last %02x, wanted 1 and 4b",
                     inv_n_got, inv_got);
            $finish;
        end
        if (inv_parity_error !== 1'b0) begin
            $display("FAIL: the inverted instance saw a parity error"); $finish;
        end

        // And it answers, which is `OE_INVERT` and `OUT_INVERT`: the
        // external stage has to pull the contact down for every zero and
        // let go for every one, or the card below reads nothing.
        wait_etu(3);
        @(negedge clk);
        while (inv_tx_ready !== 1'b1) @(negedge clk);
        inv_tx_data  = 8'h5C;
        inv_tx_valid = 1'b1;
        @(negedge line);
        card_watch(0);
        if (!parity_ok(nine, conv)) begin
            $display("FAIL: the inverted instance's parity is wrong: nine bits %b", nine);
            $finish;
        end
        if (dec_plain(nine) !== 8'h5C) begin
            $display("FAIL: the inverted instance sent %02x, wanted 5c; nine bits %b",
                     dec_plain(nine), nine);
            $finish;
        end
        repeat (4) @(negedge clk);
        // The guard is part of the character, and the counter moves one
        // cycle after it ends.
        while (inv_tx_ready !== 1'b1) @(negedge clk);
        repeat (4) @(negedge clk);
        if (inv_tx_chars !== 1 || inv_rx_chars !== 1) begin
            $display("FAIL: the inverted instance counted %0d out and %0d in, wanted 1 and 1",
                     inv_tx_chars, inv_rx_chars);
            $finish;
        end
        // The first instance was inert throughout and must have counted
        // nothing, on a contact that was busy the whole time.
        if (rx_char_count !== card_frames_sent) begin
            $display("FAIL: the inert instance counted a character off a busy contact: %0d against %0d",
                     rx_char_count, card_frames_sent);
            $finish;
        end
        inv_active = 1'b0;
        repeat (4) @(negedge clk);
        active = 1'b1;
        wait_etu(3);

        // =============================================================
        // 9. The line, idle, one last time
        // =============================================================
        set_rate(QUICK);
        wait_etu(4);
        if (io_oe !== 1'b0 || card_oe !== 1'b0 || line !== 1'b1) begin
            $display("FAIL: the idle line is io_oe=%b card_oe=%b line=%b",
                     io_oe, card_oe, line);
            $finish;
        end
        expect_counts(`__LINE__);

        $display("PASS: %0d characters out and %0d frames in over one open-drain wire, through a rate change from 5208 clocks an etu to 56 — 21505 baud to 2 Mbaud on a 112 MHz clock — back again, and on to a rate the terminal never transmitted at; in both conventions, with one frame decoded all four ways; a parity error in each direction with the pulse seen and the character repeated; %0d waiting-time expiries, one of them from a contact held low; a contact held low across reset that made no character at all; an overrun the newer character won; a run-time guard time of 4 extra etu; a divisor below DIV_MIN falling back to the parameter; the wire released within a clock of `active` falling and nothing invented when it came back on a low contact; a second instance with all three polarities inverted exchanging a character each way on the same contact; and all five counters equal to what this file drove at eighteen checkpoints",
                 term_sent, card_frames_sent, timeout_count);
        $finish;
    end
endmodule
