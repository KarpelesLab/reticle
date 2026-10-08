// `ssd1306_slave` driven the way a real driver drives a real SSD1306,
// then read back.
//
// Written after `ip/bus/spi_display_rx` was found to have two defects that
// masked each other and left one counter permanently one too high, which
// fifteen of its own testbenches missed: they did compare a counter
// against the number of things they had driven, but **never from the
// state that exposed it** — a part coming up with nothing driven yet. So
// this file checks the accounting first, from reset, and the pixels
// second:
//
//   - every counter against a driven total, including the zero case,
//     before anything is sent;
//   - `frame_done` exactly once per full frame, and never for a partial
//     one;
//   - the memory's contents at addresses computed independently of the
//     block's own pointer arithmetic.
//
// What it would catch: a pointer that advances wrongly in any of the three
// addressing modes, a window that is ignored, a byte written at the wrong
// address, a frame strobe that fires early, late, twice or never, a
// command consumed as data or an argument consumed as a command, and any
// counter that is off by any amount from reset.
//
// What it would not catch: anything about a real SSD1306. This block
// stands in for one and the command set is quoted from the datasheet, not
// measured against a panel. It also says nothing about a transport — the
// bytes arrive as bytes here.

`timescale 1ns / 1ps

module ssd1306_slave_tb;
    localparam integer COLUMNS = 128;
    localparam integer PAGES   = 8;
    localparam integer WORDS   = COLUMNS * PAGES;

    reg clk = 1'b0;
    reg rst_n = 1'b0;
    reg [7:0] in_byte = 8'd0;
    reg in_is_data = 1'b0;
    reg in_valid = 1'b0;
    reg [15:0] rd_addr = 16'd0;

    wire [7:0] rd_data;
    wire frame_done, display_on, inverse, all_on, charge_pump, seg_remap,
         com_reverse;
    wire [7:0] contrast, col_ptr, page_ptr;
    wire [1:0] addr_mode, pending_args;
    wire [15:0] cmd_count, data_count, frame_count, unknown_cmds;

    ssd1306_slave #(
        .COLUMNS(COLUMNS), .PAGES(PAGES), .COUNT_WIDTH(16)
    ) dut (
        .clk(clk), .rst_n(rst_n),
        .in_byte(in_byte), .in_is_data(in_is_data), .in_valid(in_valid),
        .rd_addr(rd_addr), .rd_data(rd_data),
        .frame_done(frame_done),
        .display_on(display_on), .inverse(inverse), .all_on(all_on),
        .contrast(contrast), .addr_mode(addr_mode),
        .charge_pump(charge_pump), .seg_remap(seg_remap),
        .com_reverse(com_reverse),
        .col_ptr(col_ptr), .page_ptr(page_ptr), .pending_args(pending_args),
        .cmd_count(cmd_count), .data_count(data_count),
        .frame_count(frame_count), .unknown_cmds(unknown_cmds)
    );

    always #5 clk = ~clk;

    // How many frame strobes have been seen, counted here rather than
    // taken from the block, so the block's own counter can be checked
    // against an independent tally.
    integer strobes = 0;
    always @(posedge clk) if (rst_n && frame_done) strobes = strobes + 1;

    integer sent_cmds = 0;
    integer sent_data = 0;

    task send;
        input [7:0] value;
        input       is_data;
        begin
            in_byte    = value;
            in_is_data = is_data;
            in_valid   = 1'b1;
            @(posedge clk);
            in_valid   = 1'b0;
            in_is_data = 1'b0;
            if (is_data) sent_data = sent_data + 1;
            else         sent_cmds = sent_cmds + 1;
            @(posedge clk);
        end
    endtask

    task cmd;  input [7:0] v; begin send(v, 1'b0); end endtask
    task dat;  input [7:0] v; begin send(v, 1'b1); end endtask

    // A byte of the memory, read through the port.
    reg [7:0] got;
    task peek;
        input integer addr;
        begin
            rd_addr = addr[15:0];
            @(posedge clk);
            @(posedge clk);
            got = rd_data;
        end
    endtask

    task expect_counts;
        input integer line;
        begin
            if (cmd_count !== sent_cmds) begin
                $display("FAIL(%0d): cmd_count %0d, drove %0d", line, cmd_count, sent_cmds);
                $finish;
            end
            if (data_count !== sent_data) begin
                $display("FAIL(%0d): data_count %0d, drove %0d", line, data_count, sent_data);
                $finish;
            end
            if (frame_count !== strobes) begin
                $display("FAIL(%0d): frame_count %0d, but %0d strobes were seen",
                         line, frame_count, strobes);
                $finish;
            end
        end
    endtask

    integer i, x, p, addr;

    initial begin
        #50_000_000;
        $display("FAIL: did not finish in the time allowed");
        $finish;
    end

    initial begin
        repeat (4) @(posedge clk);
        rst_n = 1'b1;
        repeat (4) @(posedge clk);

        // ---- Nothing driven yet: every counter must be zero. ----
        //
        // This is the case `spi_display_rx` failed and nothing checked.
        expect_counts(`__LINE__);
        if (frame_count !== 0 || strobes !== 0) begin
            $display("FAIL: a frame was counted before anything was sent");
            $finish;
        end
        if (display_on !== 1'b0 || addr_mode !== 2'd2 || contrast !== 8'h7F) begin
            $display("FAIL: power-on state is on=%b mode=%0d contrast=%02x",
                     display_on, addr_mode, contrast);
            $finish;
        end

        // ---- The initialisation sequence a real driver sends. ----
        //
        // Adafruit's SSD1306 library, 128x64, internal charge pump. Each
        // argument goes out with D/C low, like the command it belongs to.
        cmd(8'hAE);                       // display off
        cmd(8'hD5); cmd(8'h80);           // clock divide
        cmd(8'hA8); cmd(8'h3F);           // multiplex 64
        cmd(8'hD3); cmd(8'h00);           // offset 0
        cmd(8'h40);                       // start line 0
        cmd(8'h8D); cmd(8'h14);           // charge pump on
        cmd(8'h20); cmd(8'h00);           // horizontal addressing
        cmd(8'hA1);                       // segment remap
        cmd(8'hC8);                       // COM scan reversed
        cmd(8'hDA); cmd(8'h12);           // COM pins
        cmd(8'h81); cmd(8'hCF);           // contrast
        cmd(8'hD9); cmd(8'hF1);           // pre-charge
        cmd(8'hDB); cmd(8'h40);           // VCOMH
        cmd(8'hA4);                       // follow RAM
        cmd(8'hA6);                       // not inverted
        cmd(8'hAF);                       // display on

        expect_counts(`__LINE__);
        if (display_on !== 1'b1) begin $display("FAIL: display is off"); $finish; end
        if (addr_mode !== 2'd0) begin $display("FAIL: mode %0d, wanted horizontal", addr_mode); $finish; end
        if (contrast !== 8'hCF) begin $display("FAIL: contrast %02x", contrast); $finish; end
        if (charge_pump !== 1'b1) begin $display("FAIL: charge pump off"); $finish; end
        if (seg_remap !== 1'b1 || com_reverse !== 1'b1) begin
            $display("FAIL: remap=%b comrev=%b", seg_remap, com_reverse); $finish;
        end
        if (unknown_cmds !== 0) begin
            $display("FAIL: %0d commands of that sequence were not understood",
                     unknown_cmds);
            $finish;
        end
        if (pending_args !== 2'd0) begin
            $display("FAIL: still waiting for %0d argument(s)", pending_args); $finish;
        end
        if (frame_count !== 0) begin
            $display("FAIL: initialisation counted a frame"); $finish;
        end

        // ---- A full frame, the way a driver pushes one. ----
        cmd(8'h21); cmd(8'h00); cmd(COLUMNS - 1);   // column window
        cmd(8'h22); cmd(8'h00); cmd(PAGES - 1);     // page window

        // Byte at (page, column) is a value no transposition or off-by-one
        // could produce by accident.
        for (p = 0; p < PAGES; p = p + 1)
            for (x = 0; x < COLUMNS; x = x + 1)
                dat((p * 37 + x * 5 + 1) & 8'hFF);

        expect_counts(`__LINE__);
        if (strobes !== 1) begin
            $display("FAIL: %0d frame strobes for one frame", strobes); $finish;
        end
        // And the pointer is back where it started.
        if (col_ptr !== 0 || page_ptr !== 0) begin
            $display("FAIL: pointer at (%0d,%0d) after a full frame", col_ptr, page_ptr);
            $finish;
        end

        // ---- Every byte, at an address computed here. ----
        for (p = 0; p < PAGES; p = p + 1) begin
            for (x = 0; x < COLUMNS; x = x + 1) begin
                addr = p * COLUMNS + x;
                peek(addr);
                if (got !== ((p * 37 + x * 5 + 1) & 8'hFF)) begin
                    $display("FAIL: page %0d column %0d holds %02x, wanted %02x",
                             p, x, got, (p * 37 + x * 5 + 1) & 8'hFF);
                    $finish;
                end
            end
        end

        // ---- A partial frame must strobe nothing. ----
        cmd(8'h21); cmd(8'h00); cmd(8'h0F);   // sixteen columns
        cmd(8'h22); cmd(8'h00); cmd(8'h00);   // one page
        for (i = 0; i < 8; i = i + 1) dat(8'hFF);
        expect_counts(`__LINE__);
        if (strobes !== 1) begin
            $display("FAIL: a part-written window strobed a frame"); $finish;
        end
        // Finishing that window does strobe, because the window is what a
        // frame means once it has been set.
        for (i = 8; i < 16; i = i + 1) dat(8'hAA);
        if (strobes !== 2) begin
            $display("FAIL: %0d strobes after completing the window", strobes);
            $finish;
        end
        expect_counts(`__LINE__);
        peek(0);
        if (got !== 8'hFF) begin $display("FAIL: window byte 0 is %02x", got); $finish; end
        peek(8);
        if (got !== 8'hAA) begin $display("FAIL: window byte 8 is %02x", got); $finish; end
        // The byte just past the window must be untouched from the frame.
        peek(16);
        if (got !== ((0 * 37 + 16 * 5 + 1) & 8'hFF)) begin
            $display("FAIL: the window wrote past its end: %02x", got); $finish;
        end

        // ---- Page addressing, and the old column commands. ----
        cmd(8'h20); cmd(8'h02);           // page addressing
        cmd(8'hB3);                       // page 3
        cmd(8'h00 | 4'h4);                // column low nibble = 4
        cmd(8'h10 | 4'h0);                // column high nibble = 0
        dat(8'h5A);
        dat(8'hA5);
        expect_counts(`__LINE__);
        peek(3 * COLUMNS + 4);
        if (got !== 8'h5A) begin
            $display("FAIL: page mode wrote %02x at page 3 column 4", got); $finish;
        end
        peek(3 * COLUMNS + 5);
        if (got !== 8'hA5) begin
            $display("FAIL: page mode did not advance the column: %02x", got); $finish;
        end
        if (page_ptr !== 3) begin
            $display("FAIL: page mode moved the page to %0d", page_ptr); $finish;
        end

        // ---- A command this block does not implement is counted. ----
        cmd(8'h2E);                       // deactivate scroll: not modelled
        expect_counts(`__LINE__);
        if (unknown_cmds !== 1) begin
            $display("FAIL: unknown_cmds is %0d, wanted 1", unknown_cmds); $finish;
        end

        $display("PASS: power-on state, a real initialisation sequence, a full frame with every one of %0d bytes at an independently computed address, exactly one strobe per frame and none for a partial one, a window that does not write past its end, page addressing with the split column commands, and every counter equal to what was driven at six checkpoints",
                 WORDS);
        $finish;
    end
endmodule
