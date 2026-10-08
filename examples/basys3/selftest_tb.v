// `selftest.v` checked away from the board, by decoding the line it
// sends.
//
// The point of this testbench is that the self-test is itself RTL and
// can be wrong, and a self-test that is wrong reports a confident wrong
// answer — which is worse than no self-test. So the three tests that a
// simulator can run are run here, and the serial line is decoded
// character by character at the baud rate the design was built for,
// rather than trusted.
//
// WHAT CANNOT BE CHECKED HERE, and this is most of why the design
// exists: `clk_pll` and `locked` are nets nothing in the design drives —
// they are requests to the backend for a `PLLE2_BASE` and for its
// `LOCKED` output, and a simulator has no PLL to grant them. So in
// simulation the PLL count stays at zero, `locked` is unknown, and the
// status nibble that carries it is not compared. Only the part can
// answer that one, which is the whole reason the field is reported as a
// *count* rather than a verdict.
//
// The divisor is overridden to make the simulation short: at the real
// 868 cycles per bit, 34 characters take 290 microseconds of modelled
// time, which is 29 000 clock cycles for the serial line alone. `DIV` is
// a parameter of this testbench and is passed to the design, so what is
// checked here is the same RTL with a faster clock per bit.

`timescale 1ns / 1ps

module selftest_tb;
    // Cycles per bit, small enough to simulate and large enough that the
    // sampling below is not measuring its own rounding.
    localparam integer DIV = 16;

    reg clk = 1'b0;
    reg btn = 1'b0;
    reg [2:0] sw = 3'd0;

    wire led_carry, led_bram, led_lut, led_pll, led_locked, led_all,
         led_alive;
    wire [6:0] seg;
    wire       dp;
    wire [3:0] an;
    wire       tx;

    selftest #(
        .CLK_DIV     (DIV),
        // A report every 2^9 cycles rather than every 2^25.
        .TICK_BIT    (9),
        // A window of 2^6 cycles, so a passing PLL would count 2^4.
        .WINDOW_BITS (6)
    ) dut (
        .clk         (clk),
        .btn         (btn),
        .sw          (sw),
        .led_carry   (led_carry),
        .led_bram    (led_bram),
        .led_lut     (led_lut),
        .led_pll     (led_pll),
        .led_locked  (led_locked),
        .led_all     (led_all),
        .led_alive   (led_alive),
        .seg         (seg),
        .dp          (dp),
        .an          (an),
        .uart_tx_pin (tx)
    );

    always #5 clk = ~clk;

    // ---- Decoding the line ----
    //
    // Sampled in the middle of each bit, which is what a receiver does.
    // No parity, one stop bit, eight data bits, least significant first.
    integer bit_index;
    reg [7:0] ch;
    reg [8*40-1:0] line;
    integer chars;

    task get_char;
        begin
            // Wait for the start bit's falling edge.
            @(negedge tx);
            // Half a bit, to land in the middle of the start bit.
            repeat (DIV / 2) @(posedge clk);
            if (tx !== 1'b0) begin
                $display("FAIL: start bit was not low");
                $finish;
            end
            ch = 8'd0;
            for (bit_index = 0; bit_index < 8; bit_index = bit_index + 1) begin
                repeat (DIV) @(posedge clk);
                ch[bit_index] = tx;
            end
            // And the stop bit, which must be high.
            repeat (DIV) @(posedge clk);
            if (tx !== 1'b1) begin
                $display("FAIL: stop bit was not high after %02x", ch);
                $finish;
            end
        end
    endtask

    // The 32 hex characters a passing simulation must produce, with the
    // PLL's eight zeros because there is no PLL here, and with the two
    // status characters left out of the comparison.
    //
    //   EADBEEF0  the carry accumulator, 0xDEADBEEF * 16 truncated
    //   00000000  the PLL count: no PLL in simulation
    //   0000      block RAM mismatches
    //   0000      distributed RAM mismatches
    //   A5C3      the fixed word
    localparam [8*28-1:0] HEAD = "EADBEEF00000000000000000A5C3";

    integer i;
    reg [7:0] got [0:33];
    // Both characters of the sequence number: comparing only the high
    // nibble would wait sixteen reports to notice anything, which the
    // first version of this testbench did.
    reg [15:0] first_seq;

    initial begin
        // Long enough for 256 block RAM reads, the window of 2^20
        // cycles, and two reports.
        #40_000_000;
        $display("FAIL: no complete report in the time allowed");
        $finish;
    end

    initial begin
        // ---- The first line ----
        for (i = 0; i < 34; i = i + 1) begin
            get_char;
            got[i] = ch;
        end

        // The fields that do not depend on a PLL.
        for (i = 0; i < 28; i = i + 1) begin
            if (got[i] !== HEAD[8*(28-i)-1 -: 8]) begin
                $display("FAIL: character %0d is '%c' (%02x), wanted '%c'",
                         i, got[i], got[i], HEAD[8*(28-i)-1 -: 8]);
                $finish;
            end
        end

        // The line ends with CR LF.
        if (got[32] !== 8'd13 || got[33] !== 8'd10) begin
            $display("FAIL: the line does not end CR LF (%02x %02x)",
                     got[32], got[33]);
            $finish;
        end

        // The three tests a simulator can run have passed, which the
        // LEDs must agree with.
        if (led_carry !== 1'b1 || led_bram !== 1'b1 || led_lut !== 1'b1) begin
            $display("FAIL: carry=%b bram=%b lut=%b, all three should be 1",
                     led_carry, led_bram, led_lut);
            $finish;
        end
        // And the PLL's must not: there is no PLL here, so a design that
        // claimed this passed would be claiming something it cannot know.
        if (led_pll !== 1'b0) begin
            $display("FAIL: the PLL count passed with no PLL driving it");
            $finish;
        end

        first_seq = {got[30], got[31]};

        // ---- The second line, which proves it keeps running ----
        for (i = 0; i < 34; i = i + 1) begin
            get_char;
            got[i] = ch;
        end
        if ({got[30], got[31]} === first_seq) begin
            $display("FAIL: the sequence number did not advance ('%c%c')",
                     got[30], got[31]);
            $finish;
        end
        for (i = 0; i < 28; i = i + 1) begin
            if (got[i] !== HEAD[8*(28-i)-1 -: 8]) begin
                $display("FAIL: the second line differs at character %0d", i);
                $finish;
            end
        end

        $display("PASS: carry, block RAM and distributed RAM all check, the line decodes at %0d cycles per bit, and the sequence number advances", DIV);
        $finish;
    end
endmodule
