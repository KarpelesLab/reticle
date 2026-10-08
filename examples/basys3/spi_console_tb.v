// `spi_console.v` driven from both sides: SPI frames into the Pmod pins
// and commands into the serial port, with the reply decoded character by
// character.
//
// This is the first design in `examples/basys3/` that can be checked end
// to end without a person, and that is the whole reason it was built that
// way round. Everything the board reports is a number the host can
// compare, so a testbench can do exactly what the host will do: send `c`,
// read 32 hex characters, and check them against what it sent.
//
// ===================================================================
// THIS TESTBENCH CURRENTLY FAILS, AND THAT IS THE POINT
// ===================================================================
//
// It reports `FAIL: 9 frames, wanted 8`, and the fault is in
// `ip/bus/spi_display_rx`, not here. Two measurements, both from this
// file:
//
//   - `frame_count` reads **1 before a single frame has been driven**,
//     and 9 after driving 8. The byte counts are right throughout, so a
//     phantom frame carries no bits and the only symptom is one counter
//     reading one too high.
//   - The phantom comes from `gap_q` resetting to 0 while `gap` — which
//     is polarity-normalised and means "between bytes" — is 1 at idle.
//     That manufactures a `gap_rise` on the first cycle, and
//     `frame_close` is `gap_rise` with no guard.
//
// **The two defects are coupled**: `seen_gap`, which arms the block, is
// latched only inside `if (frame_close)`, so the phantom edge is also
// what arms it. Correcting `gap_q` alone drops the first byte of every
// session — measured. Arming from the gap level instead was tried and is
// **wrong**: it passes this file and fails fourteen of the block's own
// fifteen testbenches with a spurious bit error in every frame mode, for
// reasons not yet understood. So the fix needs `frame_open`,
// `counter_clear` and `close_aligned` worked through together, not one
// reset value patched at a time.
//
// Why this file found what fifteen dedicated testbenches did not: none of
// them compares `frame_count` against the number of frames driven since
// reset. They check the bytes and the error counters, which are correct.
// Testing a block's behaviour and testing its accounting are different
// jobs.
//
// The divisor is 16 cycles per bit rather than 868, and `sclk` runs at
// one tenth of the system clock rather than a quarter, so the whole
// exchange fits in a few tens of thousands of cycles. The RTL is
// otherwise the one that is built for the board.

`timescale 1ns / 1ps

module spi_console_tb;
    // Cycles per bit. Both ends use the same value through the same
    // IP, so this is only a question of how long the simulation runs.
    localparam integer DIV = 16;
    // Half an `sclk` period, in system clocks. The receiver needs at
    // least `PHASE_MARGIN` (2); five is comfortable and still quick.
    localparam integer PHASE = 5;

    reg clk = 1'b0;
    reg sclk = 1'b0, mosi = 1'b0, dc = 1'b0, cs_n = 1'b1;
    reg host_tx = 1'b1;
    wire dut_tx;
    wire [14:0] led;
    wire [6:0]  seg;
    wire        dp;
    wire [3:0]  an;

    spi_console #(
        .CLK_DIV  (DIV),
        // A report every 2^10 cycles when `a` is on, rather than 2^25.
        .TICK_BIT (10)
    ) dut (
        .clk         (clk),
        .sclk        (sclk),
        .mosi        (mosi),
        .dc          (dc),
        .cs_n        (cs_n),
        .uart_rx_pin (host_tx),
        .uart_tx_pin (dut_tx),
        .led         (led),
        .seg         (seg),
        .dp          (dp),
        .an          (an)
    );

    always #5 clk = ~clk;

    // ---- The host's side of the serial port ----
    //
    // `uart_tx` and `uart_rx` from `ip/bus/uart`, not a serial port
    // written out again here. The first version of this testbench
    // hand-rolled both halves, sampling each bit at `DIV` intervals from
    // the start edge, and misframed: it read 0x82 where the design was
    // sending ASCII hex, which cannot have bit 7 set. The same design's
    // output decoded perfectly on the real board at 868 cycles per bit,
    // so the testbench was wrong and the design was not.
    //
    // Using the library's own halves removes the guess entirely: both
    // ends agree on the framing by construction, both are covered by
    // `tests/ip_library.rs`, and what this file is actually for is
    // checking the console's logic rather than re-deriving a UART.

    // The host's own reset. `ip/bus/uart` resets on `rst_n` and has no
    // initial values, so tying this high leaves its registers undefined
    // and every byte reads as z — which is exactly how `selftest.v`
    // failed earlier today, and how this testbench failed next. A reset
    // is not optional for that IP, in a testbench either.
    reg host_rst_n = 1'b0;
    initial begin
        repeat (8) @(posedge clk);
        host_rst_n = 1'b1;
    end

    reg  [7:0] host_data  = 8'd0;
    reg        host_valid = 1'b0;
    wire       host_ready;

    uart_tx #(.CLK_DIV(DIV)) host_out (
        .clk(clk), .rst_n(host_rst_n), .div(16'd0),
        .tx_data(host_data), .tx_valid(host_valid), .tx_ready(host_ready),
        .tx(host_tx));

    wire [7:0] from_dut;
    wire       from_dut_valid;

    uart_rx #(.CLK_DIV(DIV)) host_in (
        .clk(clk), .rst_n(host_rst_n), .div(16'd0),
        .rx(dut_tx), .rx_data(from_dut), .rx_valid(from_dut_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

    task host_send;
        input [7:0] ch;
        begin
            // Present and hold until taken, which is the handshake this
            // repository's `a_streams_ready_is_a_function_of_registers`
            // rule exists to keep honest.
            host_data  = ch;
            host_valid = 1'b1;
            while (!(host_valid && host_ready)) @(posedge clk);
            @(posedge clk);
            host_valid = 1'b0;
            // Let the character finish on the wire before the next.
            repeat (DIV * 12) @(posedge clk);
        end
    endtask

    reg [7:0] ch;

    task host_recv;
        begin
            while (!from_dut_valid) @(posedge clk);
            ch = from_dut;
            @(posedge clk);
        end
    endtask

    // ---- The far side of the SPI link ----
    //
    // One byte per `cs_n`, most significant bit first, sampled by the
    // receiver on the rising edge of `sclk`, with `dc` held for the whole
    // frame — which is what a display controller does.

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

    // ---- Reading a report ----

    reg [7:0] line [0:33];
    integer   i;

    function [31:0] nib;
        input [7:0] c;
        nib = (c >= 8'd48 && c <= 8'd57)  ? (c - 8'd48)
            : (c >= 8'd65 && c <= 8'd70)  ? (c - 8'd55)
            :                               32'hFFFF_FFFF;
    endfunction

    // A field of the line, from character `at` for `n` characters.
    function [31:0] field;
        input integer at;
        input integer n;
        integer j;
        begin
            field = 32'd0;
            for (j = 0; j < n; j = j + 1) begin
                if (nib(line[at + j]) === 32'hFFFF_FFFF) begin
                    $display("FAIL: character %0d of the line is '%c', not hex",
                             at + j, line[at + j]);
                    $finish;
                end
                field = (field << 4) | nib(line[at + j]);
            end
        end
    endfunction

    task read_report;
        begin
            for (i = 0; i < 34; i = i + 1) begin
                host_recv;
                line[i] = ch;
            end
            if (line[32] !== 8'd13 || line[33] !== 8'd10) begin
                $display("FAIL: the line does not end CR LF (%02x %02x)",
                         line[32], line[33]);
                $finish;
            end
        end
    endtask

    integer cmds, datas, frames, biterr, dcchg, over, last, flags, seqn;

    task decode;
        begin
            cmds   = field(0, 4);
            datas  = field(4, 4);
            frames = field(8, 4);
            biterr = field(12, 4);
            dcchg  = field(16, 4);
            over   = field(20, 4);
            last   = field(24, 2);
            flags  = field(26, 2);
            seqn   = field(30, 2);
        end
    endtask

    initial begin
        #20_000_000;
        $display("FAIL: the exchange did not finish in the time allowed");
        $finish;
    end

    initial begin
        repeat (64) @(posedge clk);

        // Three commands and five pixels, with the last pixel the byte a
        // later question must still return.
        spi_byte(8'h2A, 1'b0);
        spi_byte(8'h2B, 1'b0);
        spi_byte(8'h2C, 1'b0);
        spi_byte(8'h11, 1'b1);
        spi_byte(8'h22, 1'b1);
        spi_byte(8'h33, 1'b1);
        spi_byte(8'h44, 1'b1);
        spi_byte(8'hA7, 1'b1);

        // Ask.
        host_send(8'h63);  // 'c'
        read_report;
        decode;

        if (cmds !== 3) begin
            $display("FAIL: %0d command bytes, wanted 3", cmds); $finish;
        end
        if (datas !== 5) begin
            $display("FAIL: %0d data bytes, wanted 5", datas); $finish;
        end
        if (frames !== 8) begin
            $display("FAIL: %0d frames, wanted 8", frames); $finish;
        end
        if (biterr !== 0) begin
            $display("FAIL: %0d bit errors, wanted 0", biterr); $finish;
        end
        if (dcchg !== 0) begin
            $display("FAIL: %0d dc changes, wanted 0", dcchg); $finish;
        end
        if (last !== 8'hA7) begin
            $display("FAIL: the last byte is %02x, wanted A7", last); $finish;
        end
        // Bit 7 of the flags is `rx_is_data`, and the last byte was a
        // pixel; bit 0 is always set so an all-zero line is impossible.
        if ((flags & 8'h80) === 0) begin
            $display("FAIL: flags %02x say the last byte was a command", flags);
            $finish;
        end
        if ((flags & 8'h01) === 0) begin
            $display("FAIL: flags %02x has the always-set bit clear", flags);
            $finish;
        end

        // `r` clears the counters, and the reply proves it rather than
        // being taken on trust.
        host_send(8'h72);  // 'r'
        read_report;
        decode;
        if (cmds !== 0 || datas !== 0 || frames !== 0) begin
            $display("FAIL: after `r` the counters read %0d/%0d/%0d",
                     cmds, datas, frames);
            $finish;
        end

        // And the link still works after a reset: one more command byte.
        spi_byte(8'h36, 1'b0);
        host_send(8'h63);
        read_report;
        decode;
        if (cmds !== 1 || frames !== 1 || last !== 8'h36) begin
            $display("FAIL: after `r` a byte gave %0d cmd, %0d frames, last %02x",
                     cmds, frames, last);
            $finish;
        end

        $display("PASS: eight frames counted as three commands and five pixels with no bit error and no dc change, the last byte read back as A7, `r` cleared every counter, and the link still received after the reset");
        $finish;
    end
endmodule
