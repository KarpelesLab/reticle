// `ssd1306_console.v` driven the way the board will be: real SSD1306
// traffic, bit by bit, into the four SPI pins — then asked over the serial
// port and checked against what was drawn.
//
// This is the end-to-end test of the whole display path:
//
//   four pins -> spi_display_rx -> ssd1306_slave -> uart -> this file
//
// Nothing is reached into. The frame buffer is read the way a host reads
// it, by sending `d` and parsing the hex, and compared against bytes this
// file computed before sending them. So a defect anywhere in that chain —
// a bit sampled on the wrong edge, a command consumed as data, a pointer
// that wraps early, a nibble emitted in the wrong order, a line break in
// the wrong place — shows up as a mismatch with an address attached.
//
// **The geometry is 16 columns by 2 pages here, not 128 by 8.** The RTL is
// the same; only the parameters differ. A full dump at 128x8 is 2064
// characters, which at any simulable divisor is some hundreds of thousands
// of cycles for one check, and the wrap, the frame strobe, the line breaks
// and the hex emitter are all exercised just as well by 32 bytes. The
// board gets the real geometry; this gets the same logic in a tenth of the
// time.
//
// The host's serial port is `uart_tx` and `uart_rx` from the library
// rather than a serial port written out again here. An earlier testbench
// in this directory hand-rolled both halves and misframed, reading 0x82
// where ASCII hex cannot have bit 7 set, and separately tied the host
// UART's `rst_n` high so every byte came back as z. Using the same IP at
// both ends removes both mistakes by construction.
//
// What this would catch: everything in the chain above that changes a byte
// or its address, plus the frame strobe firing at the wrong time, plus a
// dump that is the wrong length or shape.
//
// What it would not: anything about the four pads (nothing has been driven
// into a Pmod on this board), the real `sclk` rate, metastability, or
// whether the SSD1306 command set as implemented matches a real panel —
// that set is quoted from the datasheet and standing in for a panel is the
// point.

`timescale 1ns / 1ps

module ssd1306_console_tb;
    localparam integer DIV     = 16;   // cycles per serial bit
    localparam integer PHASE   = 5;    // system clocks per sclk half period
    localparam integer COLUMNS = 16;
    localparam integer PAGES   = 2;
    localparam integer WORDS   = COLUMNS * PAGES;
    localparam [7:0]  COL_LAST  = COLUMNS - 1;
    localparam [7:0]  PAGE_LAST = PAGES - 1;

    reg clk = 1'b0;
    reg sclk = 1'b0, mosi = 1'b0, dc = 1'b0, cs_n = 1'b1;
    wire host_tx, dut_tx;
    wire [14:0] led;
    wire [6:0]  seg;
    wire        dp;
    wire [3:0]  an;

    ssd1306_console #(
        .CLK_DIV(DIV), .TICK_BIT(10), .COLUMNS(COLUMNS), .PAGES(PAGES)
    ) dut (
        .clk(clk), .sclk(sclk), .mosi(mosi), .dc(dc), .cs_n(cs_n),
        .uart_rx_pin(host_tx), .uart_tx_pin(dut_tx),
        .led(led), .seg(seg), .dp(dp), .an(an));

    always #5 clk = ~clk;

    // ---- The host's serial port ----
    //
    // `ip/bus/uart` resets on `rst_n` and has no initial values, so tying
    // this high leaves every register at X and every byte reads as z.
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
            host_data  = ch;
            host_valid = 1'b1;
            while (!(host_valid && host_ready)) @(posedge clk);
            @(posedge clk);
            host_valid = 1'b0;
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
    // frame, which is what a display controller does.
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
    // Split in two so that `nib` is four bits wide and assigning it to a
    // four-bit slice truncates nothing.
    function is_hex;
        input [7:0] c;
        is_hex = (c >= 8'd48 && c <= 8'd57) || (c >= 8'd65 && c <= 8'd70);
    endfunction

    function [3:0] nib;
        input [7:0] c;
        nib = (c <= 8'd57) ? (c - 8'd48) : (c - 8'd55);
    endfunction

    // What was drawn, so the dump can be compared with something this file
    // computed rather than with the device's own idea of it.
    reg [7:0] drew [0:WORDS-1];

    integer i, p, x, line_len;
    reg [7:0] got_byte;

    initial begin
        #200_000_000;
        $display("FAIL: did not finish in the time allowed");
        $finish;
    end

    initial begin
        repeat (64) @(posedge clk);

        // ---- Initialise, the way a driver does ----
        cmd(8'hAE);                     // display off
        cmd(8'h20); cmd(8'h00);         // horizontal addressing
        cmd(8'h8D); cmd(8'h14);         // charge pump
        cmd(8'h81); cmd(8'hCF);         // contrast
        cmd(8'hAF);                     // display on
        cmd(8'h21); cmd(8'h00); cmd(COL_LAST);
        cmd(8'h22); cmd(8'h00); cmd(PAGE_LAST);

        // ---- Draw a frame whose bytes no off-by-one could reproduce ----
        for (p = 0; p < PAGES; p = p + 1) begin
            for (x = 0; x < COLUMNS; x = x + 1) begin
                drew[p * COLUMNS + x] = {p[3:0], x[3:0]} ^ 8'h5A;
                dat(drew[p * COLUMNS + x]);
            end
        end

        // ---- Ask for the buffer and check every byte ----
        host_send(8'h64);               // 'd'
        for (p = 0; p < PAGES; p = p + 1) begin
            line_len = 0;
            for (x = 0; x < COLUMNS; x = x + 1) begin
                host_recv;
                if (!is_hex(ch)) begin
                    $display("FAIL: page %0d byte %0d high nibble is '%c' (%02x), not hex",
                             p, x, ch, ch);
                    $finish;
                end
                got_byte[7:4] = nib(ch);
                host_recv;
                if (!is_hex(ch)) begin
                    $display("FAIL: page %0d byte %0d low nibble is '%c' (%02x), not hex",
                             p, x, ch, ch);
                    $finish;
                end
                got_byte[3:0] = nib(ch);
                if (got_byte !== drew[p * COLUMNS + x]) begin
                    $display("FAIL: page %0d column %0d dumped %02x, drew %02x",
                             p, x, got_byte, drew[p * COLUMNS + x]);
                    $finish;
                end
                line_len = line_len + 2;
            end
            // Each page is its own line.
            host_recv;
            if (ch !== 8'd13) begin
                $display("FAIL: page %0d ended with %02x, wanted CR", p, ch);
                $finish;
            end
            host_recv;
            if (ch !== 8'd10) begin
                $display("FAIL: page %0d ended CR then %02x, wanted LF", p, ch);
                $finish;
            end
            if (line_len !== COLUMNS * 2) begin
                $display("FAIL: page %0d was %0d characters, wanted %0d",
                         p, line_len, COLUMNS * 2);
                $finish;
            end
        end

        // ---- The status line, and that it agrees with the traffic ----
        host_send(8'h73);               // 's'
        // 32 hex characters, then CRLF.
        begin : status
            // One 128-bit word rather than an array of fields: an unpacked
            // array inside a named block is not supported here, and the
            // status line is a single word anyway — slicing it is closer to
            // what the design emits than reassembling it in pieces.
            reg [127:0] word;
            integer j;
            word = 128'd0;
            for (j = 0; j < 32; j = j + 1) begin
                host_recv;
                if (!is_hex(ch)) begin
                    $display("FAIL: status character %0d is '%c', not hex", j, ch);
                    $finish;
                end
                word = (word << 4) | {124'd0, nib(ch)};
            end
            host_recv;
            if (ch !== 8'd13) begin
                $display("FAIL: the status line did not end CR (%02x)", ch);
                $finish;
            end
            host_recv;
            if (ch !== 8'd10) begin
                $display("FAIL: the status line did not end LF (%02x)", ch);
                $finish;
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

        $display("PASS: a real initialisation sequence and a full %0dx%0d frame driven bit by bit into the four pins, dumped back over the serial port as %0d lines of %0d hex characters with every byte equal to what was drawn, one frame counted, %0d data bytes counted, no unknown command and no bit error",
                 COLUMNS, PAGES, PAGES, COLUMNS * 2, WORDS);
        $finish;
    end
endmodule
