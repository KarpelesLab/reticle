// Holds `smartcard_seq` (in smartcard_serial.v) to what keeps a card safe and
// to the bit rates the host asks for. Prints one PASS line, or one line per
// way it is wrong.
//
// Every cycle: while VCC is low, CLK, RST and the I/O line are low too (a
// card is never driven before it is powered, nor back-powered through a pin).
// Up: VCC, then CLK after VCC_SETTLE, then RST after RST_HOLD card clocks.
// Down: RST, then CLK, then I/O, then VCC. And 60 MHz / baud, rounded.
`timescale 1ns / 1ps
module smartcard_seq_tb;
    localparam integer CARD_DIV = 8, VCC_SETTLE = 50, RST_HOLD = 10, DEACT_STEP = 20;

    reg clk = 1'b0;
    always #8 clk = ~clk;
    reg rst_n = 1'b0, dtr = 1'b0, configured = 1'b0;
    reg [31:0] baud = 32'd0;
    wire card_vcc, card_rst, card_clk, io_on, running;
    wire [15:0] etu_div;

    smartcard_seq #(
        .CARD_DIV(CARD_DIV), .VCC_SETTLE(VCC_SETTLE), .RST_HOLD(RST_HOLD), .DEACT_STEP(DEACT_STEP)
    ) dut (
        .clk(clk), .rst_n(rst_n), .dtr(dtr), .configured(configured), .baud(baud),
        .card_vcc(card_vcc), .card_rst(card_rst), .card_clk(card_clk),
        .io_on(io_on), .running(running), .etu_div(etu_div)
    );

    integer errors = 0;
    task check(input bad, input [8*72-1:0] what);
        if (bad) begin
            $display("FAIL: %0s", what);
            errors = errors + 1;
        end
    endtask

    // The invariant, every cycle.
    always @(posedge clk) if (rst_n) begin
        if (!card_vcc && (card_clk || card_rst || io_on)) begin
            $display("FAIL: a line is up while VCC is off (clk %b rst %b io %b) at %0t",
                     card_clk, card_rst, io_on, $time);
            errors = errors + 1;
        end
        if (card_rst && !running) begin
            $display("FAIL: RST is released while the card is not running at %0t", $time);
            errors = errors + 1;
        end
    end

    // Cycle counter and the moments each line changes.
    integer cyc = 0;
    always @(posedge clk) cyc <= cyc + 1;
    integer t_vcc_up, t_clk_up, t_rst_up, t_rst_dn, t_clk_dn, t_io_dn, t_vcc_dn;
    reg prev_vcc = 0, prev_rst = 0, prev_io = 0, clk_seen = 0;
    integer last_clk_edge = 0;
    always @(posedge clk) begin
        if (card_vcc && !prev_vcc) t_vcc_up = cyc;
        if (!card_vcc && prev_vcc) t_vcc_dn = cyc;
        if (card_rst && !prev_rst) t_rst_up = cyc;
        if (!card_rst && prev_rst) t_rst_dn = cyc;
        if (!io_on && prev_io) t_io_dn = cyc;
        if (card_clk && !clk_seen) begin t_clk_up = cyc; clk_seen = 1; end
        if (card_clk) last_clk_edge = cyc;
        prev_vcc <= card_vcc; prev_rst <= card_rst; prev_io <= io_on;
    end

    // The card clock's period and duty, measured while it runs.
    integer highs = 0, lows = 0;
    reg counting = 0;
    always @(posedge clk) if (counting) begin
        if (card_clk) highs = highs + 1; else lows = lows + 1;
    end

    task rate(input [31:0] b, input [15:0] want, input [8*72-1:0] what);
        begin
            baud = b;
            repeat (40) @(posedge clk);
            if (etu_div !== want) begin
                $display("FAIL: %0s: baud %0d gave etu %0d, want %0d", what, b, etu_div, want);
                errors = errors + 1;
            end
        end
    endtask

    initial begin
        repeat (4) @(posedge clk);
        rst_n = 1'b1;
        repeat (4) @(posedge clk);
        check(card_vcc || card_clk || card_rst || io_on, "a line is up after reset");
        check(etu_div !== 372 * CARD_DIV, "the reset rate is not the ATR's");

        // DTR alone, before USB is configured, powers nothing.
        dtr = 1'b1;
        repeat (100) @(posedge clk);
        check(card_vcc, "VCC came up before USB was configured");

        // ---- up ----
        configured = 1'b1;
        repeat (VCC_SETTLE + RST_HOLD * CARD_DIV + 40) @(posedge clk);
        check(!card_vcc || !running || !card_rst, "the card is not up");
        check(t_clk_up - t_vcc_up < VCC_SETTLE, "CLK started before VCC had settled");
        check(t_rst_up - t_clk_up < RST_HOLD * CARD_DIV - CARD_DIV,
              "RST was released before RST_HOLD card clocks");

        counting = 1;
        repeat (CARD_DIV * 20) @(posedge clk);
        counting = 0;
        check(highs !== lows, "the card clock's duty is not 50 %");
        check(highs + lows !== CARD_DIV * 20, "the duty count lost cycles");

        // ---- the rates a host asks for ----
        rate(32'd20161,   16'd2976, "the ATR's rate at 7.5 MHz");
        rate(32'd1875000, 16'd32,   "PPS1 87, F/D 4");
        rate(32'd175781,  16'd341,  "PPS1 98, Fi 512 Di 12");
        rate(32'd9600,    16'd6250, "9600");
        rate(32'd26882,   16'd2232, "rounded, not truncated (2231.97)");
        rate(32'd100,     16'd2232, "an out-of-range rate changes nothing");

        // ---- down ----
        dtr = 1'b0;
        repeat (3 * DEACT_STEP + 40) @(posedge clk);
        check(card_vcc || card_rst || io_on || running, "the card is not down");
        check(!(t_rst_dn < last_clk_edge), "CLK stopped before RST dropped");
        check(!(last_clk_edge < t_io_dn), "I/O was driven low while CLK still ran");
        check(!(t_io_dn < t_vcc_dn), "VCC dropped before I/O");
        check(card_clk, "CLK is high with the card off");

        // And up again: a second activation from scratch.
        clk_seen = 0;
        dtr = 1'b1;
        repeat (VCC_SETTLE + RST_HOLD * CARD_DIV + 40) @(posedge clk);
        check(!running, "a second activation failed");
        configured = 1'b0;
        repeat (3 * DEACT_STEP + 40) @(posedge clk);
        check(card_vcc, "losing USB did not power the card down");

        if (errors == 0) $display("PASS: the card is powered, clocked and released in order, and dropped in reverse");
        $finish;
    end
endmodule
