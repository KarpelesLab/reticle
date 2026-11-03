// `ssd1306_console.v` driven by a driver with **no `dc` wire**: the pin is
// left at `z`, and `ip/bus/spi_display_rx` has to tell commands from pixels
// by where each byte falls. README.md §3b there is the account.
//
// The traffic is the shape inference is declared for: an initialisation
// sequence longer than any prefix, then runs of a six-byte address window and
// a whole frame. Two runs follow each other with no delay, which only the
// declared length can separate, and a third follows a delay.
//
// The core's bytes are taken directly rather than through a serial port,
// because `ssd1306_console_tb.v` already checks the port and the same dump.
//
// What this would catch: a window command written as pixels, or a pixel run
// as a command, which moves the dump, the frame count, the data count or the
// unknown-command count; and an undriven pin reaching a byte or a tag, since
// `z` would come out as X or withhold it.
//
// What it would not: whether a given driver sends runs of this shape. Its
// numbers are read from the same driver with `dc` connected.

`timescale 1ns / 1ps

module ssd1306_nodc_tb;
    localparam integer PHASE   = 5;    // system clocks per sclk half period
    localparam integer COLUMNS = 16;
    localparam integer PAGES   = 2;
    localparam integer WORDS   = COLUMNS * PAGES;
    localparam [7:0]  COL_LAST  = COLUMNS - 1;
    localparam [7:0]  PAGE_LAST = PAGES - 1;
    // Four times the 25 clocks between one byte's last `sclk` edge and the
    // next's first, and well under the delay the third run follows.
    localparam integer IDLE    = 100;

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
        .DC_INFER(1), .INFER_INIT_BYTES(8), .INFER_CMD_BYTES(6),
        .INFER_DATA_BYTES(WORDS), .INFER_IDLE_CLOCKS(IDLE)
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

    integer i;
    task send_run;
        input [7:0] seed;
        begin
            spi_byte(8'h21); spi_byte(8'h00); spi_byte(COL_LAST);
            spi_byte(8'h22); spi_byte(8'h00); spi_byte(PAGE_LAST);
            for (i = 0; i < WORDS; i = i + 1) begin
                drew[i] = i[7:0] * 8'd3 + seed;
                spi_byte(drew[i]);
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
        repeat (64) @(posedge clk);

        // ---- Initialise: eight bytes, and a pause no prefix would survive ----
        spi_byte(8'hAE);                // display off
        spi_byte(8'h20); spi_byte(8'h00); // horizontal addressing
        repeat (IDLE * 3) @(posedge clk);
        spi_byte(8'h8D); spi_byte(8'h14); // charge pump
        spi_byte(8'h81); spi_byte(8'hCF); // contrast
        spi_byte(8'hAF);                // display on

        // ---- Two runs back to back, and a third after a delay ----
        // Checked after each of the last two, because a later frame covers
        // whatever an earlier one got wrong.
        send_run(8'h17);
        send_run(8'h51);
        check_dump;
        repeat (IDLE * 3) @(posedge clk);
        send_run(8'h8B);
        check_dump;

        // ---- And the accounting ----
        request(8'h3F);                 // '?'
        read_line;
        if (line_len !== 32) begin
            $display("FAIL: the status line was %0d characters", line_len);
            $finish;
        end
        if ({byte_at(0), byte_at(1)} !== 16'd3) begin
            $display("FAIL: %0d frames completed, wanted 3",
                     {byte_at(0), byte_at(1)});
            $finish;
        end
        if ({byte_at(2), byte_at(3)} !== 3 * WORDS) begin
            $display("FAIL: %0d data bytes, drew %0d",
                     {byte_at(2), byte_at(3)}, 3 * WORDS);
            $finish;
        end
        if ({byte_at(4), byte_at(5)} !== 8 + 3 * 6) begin
            $display("FAIL: %0d command bytes, sent %0d",
                     {byte_at(4), byte_at(5)}, 8 + 3 * 6);
            $finish;
        end
        if ({byte_at(6), byte_at(7)} !== 16'd0) begin
            $display("FAIL: %0d command(s) were not understood",
                     {byte_at(6), byte_at(7)});
            $finish;
        end

        $display("PASS: with `dc` undriven, an initialisation and three %0dx%0d frames landed where they were drawn, %0d commands and %0d data bytes counted and no command misunderstood",
                 COLUMNS, PAGES, 8 + 3 * 6, 3 * WORDS);
        $finish;
    end
endmodule
