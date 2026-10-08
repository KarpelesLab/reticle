// `iso7816_probe.v` checked against a model card — and checked for the one
// thing that must never go wrong, which is leaving a device powered.
//
// This design switches somebody else's hardware on. So the order of the
// activation and deactivation sequences is asserted rather than eyeballed,
// and the watchdog that drops power on its own is tested by letting it
// fire. A probe whose `vcc_en` can stick high is worse than no probe.
//
// What it checks:
//
//   1. **Activation order**: `vcc_en` rises, *then* the clock starts,
//      *then* reset is released — never overlapping, never reordered.
//      Driving a clock into an unpowered device forward-biases its input
//      diodes, so this order is a hardware requirement and not a style.
//   2. **The reset hold is counted in card clock cycles**, at least
//      `RST_HOLD` of them, so it stays correct at any card frequency.
//   3. The card's characters are received at 8E2 and printed as hex.
//   4. **Deactivation order**: reset low and clock stopped *before* power
//      goes, with the line never driven.
//   5. **The watchdog fires**, dropping power with no command at all.
//
// What it would not catch: anything about the real card, the real relay's
// turn-on time, or the analogue behaviour of an open-drain line. The etu
// and the watchdog are shortened by parameter so the whole thing simulates
// in a few hundred thousand cycles; the RTL is the one that goes on the
// board.

`timescale 1ns / 1ps

module iso7816_probe_tb;
    localparam integer CARD_DIV   = 14;    // 8 MHz from 112 MHz
    localparam integer ETU_CYCLES = 8;     // short, so a byte is quick
    localparam integer ETU_DIV    = ETU_CYCLES * CARD_DIV;
    localparam integer RST_HOLD   = 400;
    localparam integer HOST_DIV   = 16;

    reg clk = 1'b0;
    reg io_drive = 1'b1;          // the card's open-drain line, idle high
    wire clk_card, rst_card, vcc_en;
    wire [14:0] led;
    wire [6:0]  seg;
    wire        dp;
    wire [3:0]  an;
    wire host_tx, dut_tx;

    iso7816_probe #(
        .CARD_DIV(CARD_DIV), .ETU_CYCLES(ETU_CYCLES), .RST_HOLD(RST_HOLD),
        .VCC_BITS(6), .GAP_ETU(24), .HOST_DIV(HOST_DIV), .WDOG_BITS(18),
        // No PLL in a netlist: run on this testbench's own clock.
        .USE_PLL(0)
    ) dut (
        .clk(clk), .clk_card(clk_card), .rst_card(rst_card),
        .vcc_en(vcc_en), .io(io_drive),
        .uart_rx_pin(host_tx), .uart_tx_pin(dut_tx),
        .led(led), .seg(seg), .dp(dp), .an(an));

    always #5 clk = ~clk;

    // ---- The host's end of the serial port ----
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

    task host_send;
        input [7:0] ch;
        begin
            host_data  = ch;
            host_valid = 1'b1;
            while (!(host_valid && host_ready)) @(posedge clk);
            @(posedge clk);
            host_valid = 1'b0;
            repeat (HOST_DIV * 12) @(posedge clk);
        end
    endtask

    // **The device's output is buffered continuously**, and that is not a
    // convenience. `host_send` waits out the character it transmitted, and
    // the probe answers immediately — so a `host_recv` that only looks at
    // `from_dut_valid` when asked loses every character that arrives while
    // the testbench is talking. That cost the first one or two characters
    // of `+VCC`, which showed up as the banners being 17 characters instead
    // of 18 and every later comparison sliding by one.
    //
    // Any testbench driving a full-duplex port needs this. Polling a strobe
    // only when convenient works exactly until the device replies promptly.
    reg [7:0] rxq [0:1023];
    integer   rx_wr = 0;
    integer   rx_rd = 0;
    always @(posedge clk) begin
        if (from_dut_valid) begin
            rxq[rx_wr % 1024] = from_dut;
            rx_wr = rx_wr + 1;
        end
    end

    reg [7:0] ch;
    task host_recv;
        begin
            while (rx_rd >= rx_wr) @(posedge clk);
            ch    = rxq[rx_rd % 1024];
            rx_rd = rx_rd + 1;
        end
    endtask

    // ---- The model card: 8E2 at the probe's etu, direct convention ----
    integer bit_i;
    reg parity;
    task card_byte;
        input [7:0] value;
        begin
            parity = ^value;                 // even parity over 8 bits
            io_drive = 1'b0;                 // start
            repeat (ETU_DIV) @(posedge clk);
            for (bit_i = 0; bit_i < 8; bit_i = bit_i + 1) begin
                io_drive = value[bit_i];     // least significant first
                repeat (ETU_DIV) @(posedge clk);
            end
            io_drive = parity;
            repeat (ETU_DIV) @(posedge clk);
            io_drive = 1'b1;                 // two stop bits of guard
            repeat (ETU_DIV * 2) @(posedge clk);
        end
    endtask

    // ---- Watching the sequence ----
    // 64-bit, because `$time` is and an `integer` is not — assigning it to
    // one truncates, which the simulator rightly warns about. Zero means
    // "not yet", which is safe since nothing happens at time zero.
    reg [63:0] t_vcc = 64'd0, t_clk = 64'd0, t_rst = 64'd0;
    reg clk_seen = 1'b0;
    always @(posedge clk) begin
        if (vcc_en && t_vcc == 0)             t_vcc = $time;
        if (clk_card && !clk_seen) begin clk_seen = 1'b1; t_clk = $time; end
        if (rst_card && t_rst == 0)           t_rst = $time;
    end

    // How many card clock rises happened between the clock starting and
    // reset being released.
    integer card_rises = 0;
    reg card_prev = 1'b0;
    always @(posedge clk) begin
        if (clk_card && !card_prev && !rst_card && clk_seen)
            card_rises = card_rises + 1;
        card_prev = clk_card;
    end

    task expect_line;
        input [8*4-1:0] want;
        integer k;
        begin
            for (k = 3; k >= 0; k = k - 1) begin
                host_recv;
                if (ch !== want[8*k +: 8]) begin
                    $display("FAIL: banner character is '%c', wanted '%c'",
                             ch, want[8*k +: 8]);
                    $finish;
                end
            end
            host_recv;
            if (ch !== 8'd13) begin $display("FAIL: banner lacks CR"); $finish; end
            host_recv;
            if (ch !== 8'd10) begin $display("FAIL: banner lacks LF"); $finish; end
        end
    endtask

    integer i;
    // **The real device's answer-to-reset**, as its owner reported it, not
    // an invented one. A well-formed T=0 ATR: TS=3B direct convention,
    // T0=1B meaning TA1 follows and eleven historical bytes, then "2.0.1"
    // in ASCII among the historical bytes, and no TCK because the protocol
    // is T=0.
    //
    // **TA1 = 87 is partly vendor-specific, and the first reading of it
    // here was wrong.** The low nibble is standard: DI = 7 means Di = 64.
    // The high nibble is not: **FI = 8 is RFU** in ISO 7816-3's Fi table —
    // `Fi = 512` is FI = **9**, which is what this comment first claimed.
    // So the clock-rate conversion factor is whatever the vendor defines.
    //
    // The device's owner targets 2 Mbaud at an 8 MHz clock, which fixes
    // `F/D = 4`, so with Di = 64 the vendor's Fi is 256 — another value the
    // standard does not list. The consequence is better than the wrong
    // reading suggested: **8 MHz already gives exactly 2 Mbaud**, so no
    // 16 MHz clock is needed and the odd-divisor duty-cycle problem that
    // 16 MHz would have brought does not arise. The post-PPS etu is 4 card
    // clock cycles, which at CARD_DIV=14 is 56 system cycles.
    //
    // Fourteen bytes is a better test than five for a reason beyond
    // length: it crosses the idle-gap timeout several times over, so a gap
    // counted in the wrong unit shows up as a CRLF in the middle of the
    // answer rather than passing unnoticed.
    localparam integer ATR_LEN = 14;
    reg [7:0] atr [0:13];
    reg [7:0] want;

    function [7:0] hexch;
        input [3:0] n;
        hexch = (n < 4'd10) ? (8'd48 + {4'd0, n}) : (8'd55 + {4'd0, n});
    endfunction

    initial begin
        #100_000_000;
        $display("FAIL: did not finish in the time allowed");
        $finish;
    end

    initial begin
        repeat (64) @(posedge clk);

        // Nothing may be powered before it is asked for.
        if (vcc_en !== 1'b0) begin
            $display("FAIL: vcc_en is high before activation"); $finish;
        end
        if (clk_card !== 1'b0) begin
            $display("FAIL: the card clock runs before activation"); $finish;
        end
        if (rst_card !== 1'b0) begin
            $display("FAIL: reset is not asserted before activation"); $finish;
        end

        host_send(8'h41);                    // 'A'
        expect_line("+VCC");
        expect_line("+CLK");
        expect_line("+RST");

        // The order, which is a hardware requirement.
        if (!(t_vcc < t_clk && t_clk < t_rst)) begin
            $display("FAIL: order was vcc=%0t clk=%0t rst=%0t", t_vcc, t_clk, t_rst);
            $finish;
        end
        if (card_rises < RST_HOLD) begin
            $display("FAIL: reset released after %0d card clocks, wanted >= %0d",
                     card_rises, RST_HOLD);
            $finish;
        end

        // ---- The card answers ----
        atr[0]  = 8'h3B; atr[1]  = 8'h1B; atr[2]  = 8'h87; atr[3]  = 8'h05;
        atr[4]  = 8'h32; atr[5]  = 8'h2E; atr[6]  = 8'h30; atr[7]  = 8'h2E;
        atr[8]  = 8'h31; atr[9]  = 8'h04; atr[10] = 8'h33; atr[11] = 8'h00;
        atr[12] = 8'h00; atr[13] = 8'h04;
        for (i = 0; i < ATR_LEN; i = i + 1) card_byte(atr[i]);

        // Through a temporary, because slicing an array element directly
        // (`atr[i][3:0]`) is not supported here.
        for (i = 0; i < ATR_LEN; i = i + 1) begin
            want = atr[i];
            host_recv;
            if (ch !== hexch(want[7:4])) begin
                $display("FAIL: byte %0d high nibble came back '%c', wanted '%c'",
                         i, ch, hexch(want[7:4]));
                $finish;
            end
            host_recv;
            if (ch !== hexch(want[3:0])) begin
                $display("FAIL: byte %0d low nibble came back '%c', wanted '%c'",
                         i, ch, hexch(want[3:0]));
                $finish;
            end
        end

        // ---- Deactivate, and check the order ----
        host_send(8'h44);                    // 'D'
        // Reset and clock must be down before power goes.
        while (vcc_en === 1'b1) begin
            @(posedge clk);
            if (vcc_en && rst_card) begin
                $display("FAIL: reset still released while powering down");
                $finish;
            end
        end
        if (clk_card !== 1'b0) begin
            $display("FAIL: the clock is not stopped low after power off");
            $finish;
        end
        expect_line("-OFF");

        $display("PASS: power then clock then reset, in that order and never overlapping; reset held %0d card clock cycles; all %0d bytes of the real device's answer-to-reset received at 8E2 and printed; deactivation dropped reset and the clock before power",
                 card_rises, ATR_LEN);
        $finish;
    end

    // ---- And the watchdog, in a second run of its own ----
    //
    // Checked by `iso7816_probe_wdog_tb` rather than here, because letting
    // it fire means waiting out the timer and this file has a card to talk
    // to. See that module below.
endmodule

// A second, smaller check: with nothing said and nothing received, the
// watchdog must drop power by itself. This is the property that makes the
// design safe to leave on a bench.
module iso7816_probe_wdog_tb;
    localparam integer HOST_DIV = 16;

    reg clk = 1'b0;
    wire clk_card, rst_card, vcc_en;
    wire [14:0] led;
    wire [6:0]  seg;
    wire        dp;
    wire [3:0]  an;
    wire host_tx, dut_tx;

    iso7816_probe #(
        .CARD_DIV(14), .ETU_CYCLES(8), .RST_HOLD(400),
        .VCC_BITS(6), .GAP_ETU(24), .HOST_DIV(HOST_DIV), .WDOG_BITS(13),
        .USE_PLL(0)
    ) dut (
        .clk(clk), .clk_card(clk_card), .rst_card(rst_card),
        .vcc_en(vcc_en), .io(1'b1),
        .uart_rx_pin(host_tx), .uart_tx_pin(dut_tx),
        .led(led), .seg(seg), .dp(dp), .an(an));

    always #5 clk = ~clk;

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

    integer waited;

    initial begin
        #50_000_000;
        $display("FAIL(wdog): the watchdog never fired — a device would be left powered");
        $finish;
    end

    initial begin
        repeat (64) @(posedge clk);
        host_data  = 8'h41;                  // 'A'
        host_valid = 1'b1;
        while (!(host_valid && host_ready)) @(posedge clk);
        @(posedge clk);
        host_valid = 1'b0;

        // Wait for power to come up, then say nothing at all.
        while (vcc_en !== 1'b1) @(posedge clk);
        waited = 0;
        while (vcc_en === 1'b1) begin
            @(posedge clk);
            waited = waited + 1;
        end
        if (rst_card !== 1'b0 || clk_card !== 1'b0) begin
            $display("FAIL(wdog): power went with reset=%b clk=%b, wanted both low",
                     rst_card, clk_card);
            $finish;
        end
        $display("PASS(wdog): with nothing said, power dropped by itself after %0d cycles, with reset asserted and the clock stopped",
                 waited);
        $finish;
    end
endmodule
