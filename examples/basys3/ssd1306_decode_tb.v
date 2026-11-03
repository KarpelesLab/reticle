// `ssd1306_console.v` with `dc` left at `z` and the tag read from the commands
// by `ip/video/ssd1306_dc_decode`, on the traffic the user reported from their
// device: a nine-byte prefix, then for each page one command and a page of
// pixels. The geometry is 16x2, for the reason `ssd1306_console_tb.v` gives.
//
// What this would catch: an argument or a pixel read as a command, which moves
// the dump or the counts; a prefix treated as fixed; a short draw not recovered;
// and an undriven pin reaching a byte.
//
// What it would not: anything the real device does that was not reported.

`timescale 1ns / 1ps

module ssd1306_decode_tb;
    localparam integer PHASE   = 5;    // system clocks per sclk half period
    localparam integer COLUMNS = 16;
    localparam integer PAGES   = 2;
    localparam integer WORDS   = COLUMNS * PAGES;
    localparam [7:0]  COL_LAST  = COLUMNS - 1;
    localparam [7:0]  PAGE_LAST = PAGES - 1;
    // Longer than anything inside a frame, shorter than the delays below.
    localparam integer IDLE    = 100;
    // Two prefixes and the third frame's three commands, a short draw's page
    // command, and a page command per page per frame.
    localparam integer COMMANDS = 9 + 3 + 3 + 1 + 3 * PAGES;

    reg clk = 1'b0;
    reg rst_n = 1'b0;
    reg sclk = 1'b0, mosi = 1'b0, cs_n = 1'b1;
    // Nothing drives this and nothing should need to.
    wire dc = 1'bz;

    reg  [7:0] cmd_data  = 8'd0;
    reg        cmd_valid = 1'b0;
    wire [7:0] out_data;
    wire       out_valid;

    ssd1306_console #(
        .TICK_BIT(10), .COLUMNS(COLUMNS), .PAGES(PAGES),
        .DC_DECODE(1), .DECODE_IDLE_CLOCKS(IDLE)
    ) dut (
        .clk(clk), .rst_n(rst_n),
        .sclk(sclk), .mosi(mosi), .dc(dc), .cs_n(cs_n),
        .cmd_valid(cmd_valid), .cmd_data(cmd_data),
        .out_valid(out_valid), .out_data(out_data), .out_ready(1'b1),
        .press_left(), .press_right(),
        .led(), .seg(), .dp(), .an());

    always #5 clk = ~clk;

    integer b;
    task spi_byte;
        input [7:0] value;
        begin
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

    reg [7:0] drew [0:WORDS-1];

    // What a frame draws: bytes that mean something as commands, because a
    // pixel read as one would act on it.
    function [7:0] pixel;
        input integer index;
        input [7:0] seed;
        begin
            case ((index + seed) % 6)
                0: pixel = 8'hB0;
                1: pixel = 8'h21;
                2: pixel = 8'hAE;
                3: pixel = 8'h00;
                4: pixel = 8'h10;
                default: pixel = 8'h5A + index[7:0];
            endcase
        end
    endfunction

    // The device's frame after its prefix: for each page one command and a
    // page of pixels.
    integer q, y;
    task send_pages;
        input [7:0] seed;
        begin
            for (q = 0; q < PAGES; q = q + 1) begin
                spi_byte(8'hB0 + q[7:0]);
                for (y = 0; y < COLUMNS; y = y + 1) begin
                    drew[q * COLUMNS + y] = pixel(q * COLUMNS + y, seed);
                    spi_byte(drew[q * COLUMNS + y]);
                end
            end
        end
    endtask

    // Both answers arrive as hex with a CRLF: this gathers one line.
    reg [8*80-1:0] line;
    integer line_len;
    task request;
        input [7:0] letter;
        begin
            cmd_data  = letter;
            cmd_valid = 1'b1;
            @(posedge clk);
            cmd_valid = 1'b0;
        end
    endtask

    task read_line;
        integer waited;
        reg done;
        begin
            line = 0;
            line_len = 0;
            done = 1'b0;
            waited = 0;
            while (!done) begin
                @(posedge clk);
                waited = waited + 1;
                if (waited > 1000000) begin
                    $display("FAIL: no answer from the core");
                    $finish;
                end
                if (out_valid) begin
                    if (out_data == 8'd10) begin
                        done = 1'b1;
                    end else if (out_data != 8'd13) begin
                        line = (line << 8) | out_data;
                        line_len = line_len + 1;
                    end
                end
            end
        end
    endtask

    function [3:0] nib;
        input [7:0] c;
        nib = (c <= 8'd57) ? (c - 8'd48) : (c - 8'd55);
    endfunction

    // The status word, or a byte of a dumped page, by character position
    // from the left of the line just read.
    function [7:0] byte_at;
        input integer index;
        reg [7:0] hi, lo;
        begin
            hi = line >> (8 * (line_len - 1 - 2 * index));
            lo = line >> (8 * (line_len - 2 - 2 * index));
            byte_at = {nib(hi), nib(lo)};
        end
    endfunction

    // The whole buffer must be the frame last drawn.
    integer p, x;
    task check_dump;
        begin
            request(8'h67);             // 'g'
            for (p = 0; p < PAGES; p = p + 1) begin
                read_line;
                if (line_len !== COLUMNS * 2) begin
                    $display("FAIL: page %0d was %0d characters", p, line_len);
                    $finish;
                end
                for (x = 0; x < COLUMNS; x = x + 1) begin
                    if (byte_at(x) !== drew[p * COLUMNS + x]) begin
                        $display("FAIL: page %0d column %0d dumped %02x, drew %02x",
                                 p, x, byte_at(x), drew[p * COLUMNS + x]);
                        $finish;
                    end
                end
            end
        end
    endtask

    initial begin
        #100_000_000;
        $display("FAIL: did not finish in the time allowed");
        $finish;
    end

    initial begin
        repeat (8) @(posedge clk);
        rst_n = 1'b1;
        // The first silence is what lets the decoder start: a driver talking
        // within IDLE clocks of reset loses what it sent until its next pause,
        // which is what the first run of this bench did.
        repeat (IDLE * 2) @(posedge clk);

        // ---- The device's nine-byte prefix, whose arguments look like a
        //      page command and a window ----
        spi_byte(8'hDB); spi_byte(8'hB0);
        spi_byte(8'hD5); spi_byte(8'h21);
        spi_byte(8'h81); spi_byte(8'hB7);
        spi_byte(8'hA6); spi_byte(8'hC0); spi_byte(8'hA1);
        send_pages(8'd0);
        repeat (IDLE * 3) @(posedge clk);
        check_dump;

        // ---- A different prefix, which no count could have followed ----
        spi_byte(8'hAF);
        spi_byte(8'h8D); spi_byte(8'h14);
        send_pages(8'd2);
        repeat (IDLE * 3) @(posedge clk);
        check_dump;

        // ---- A draw cut short, then a whole frame after the silence ----
        spi_byte(8'hB0);
        spi_byte(8'h77); spi_byte(8'h77);
        repeat (IDLE * 3) @(posedge clk);
        // The display's column really is 2 now, as a panel's would be, so a
        // driver that wants column 0 says so. The first run of this bench did
        // not, and read the shift as a fault.
        spi_byte(8'hA6);
        spi_byte(8'h00); spi_byte(8'h10);
        send_pages(8'd4);
        repeat (IDLE * 3) @(posedge clk);
        check_dump;

        // ---- And the accounting ----
        request(8'h3F);                 // '?'
        read_line;
        if (line_len !== 32) begin
            $display("FAIL: the status line was %0d characters", line_len);
            $finish;
        end
        // Three whole frames and two pixels.
        if ({byte_at(0), byte_at(1)} !== 16'd3) begin
            $display("FAIL: %0d frames, drew 3", {byte_at(0), byte_at(1)});
            $finish;
        end
        if ({byte_at(2), byte_at(3)} !== 3 * WORDS + 2) begin
            $display("FAIL: %0d data bytes, drew %0d",
                     {byte_at(2), byte_at(3)}, 3 * WORDS + 2);
            $finish;
        end
        if ({byte_at(4), byte_at(5)} !== COMMANDS) begin
            $display("FAIL: %0d command bytes, sent %0d",
                     {byte_at(4), byte_at(5)}, COMMANDS);
            $finish;
        end
        if ({byte_at(6), byte_at(7)} !== 16'd0) begin
            $display("FAIL: %0d command(s) were not understood",
                     {byte_at(6), byte_at(7)});
            $finish;
        end

        $display("PASS: with `dc` undriven, the device's frame, a changed prefix and recovery from a short draw landed as drawn, with %0d commands and %0d data bytes counted and no command misunderstood",
                 COMMANDS, 3 * WORDS + 2);
        $finish;
    end
endmodule
