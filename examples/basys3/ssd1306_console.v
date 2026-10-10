// A Digilent Basys 3 pretending to be a 128x64 SSD1306, with the pixels
// readable over the board's serial port.
//
// Two blocks and a pair of byte streams:
//
//   ip/bus/spi_display_rx    the four wires -> bytes with a D/C tag
//   ip/video/ssd1306_slave   those bytes -> the command set and GDDRAM
//
// Plug a driver's SPI output into Pmod JA, open the serial port at 115200,
// and read the screen. `frame_done` from the slave says when a whole frame
// has landed, so `o` streams a copy after every redraw without a host
// having to guess at timing.
//
// **This module has no serial port and no tristate of its own.** It takes
// received bytes and gives characters to send, and asks for a button press
// with a plain output that whoever owns the pins turns into a released or
// driven line. `ssd1306_console_pad.v` is that owner for a board doing
// nothing else; `iso_display.v` is the one that matters — there this core
// and `iso7816_terminal.v` share the board's single serial port through an
// arbiter, because a card that must be initialised over ISO 7816 and then
// driven over SPI cannot be split across two bitstreams: reloading the part
// drops `vcc_en` and power-cycles the device.
//
// ===================================================================
// THE CONVERSATION
// ===================================================================
//
//   ?   status: one line of 32 hex characters (below)
//   g   get the frame buffer: **8 lines of 256 hex characters**, one
//       line per page, 1024 bytes in all
//   o   dump after every completed frame, unprompted ("on")
//   n   stop dumping unprompted ("no")
//   z   zero the SPI receiver and the display. It answers nothing — ask
//       with `?` — because only `?` reports status now; see below.
//   <   press the far side's LEFT button for `2**PRESS_BITS` clocks
//   >   the same for its RIGHT button
//
// **These letters were moved, and `s d a q r L R` are what they were.**
// Sharing a port with `iso7816_terminal.v` means sharing an alphabet, and
// that half's `A P F S D s : L C k` are documented, measured against a real
// card and learned by a person at a terminal, so this half moved. The rule
// the new set obeys: **no display command is a hexadecimal digit**, which
// is what makes a `:`-prefixed run of card bytes unable to contain one, and
// none of them is an ISO letter. `iso_display.v` leans on both facts.
//
// **Only `?` reports status now.** It used to be "anything else reports
// status, so a stray newline costs a line and never silence" — which is
// wrong the moment two cores share a port: every `A`, `P` or `F` meant for
// the card half, and every CR and LF a host sends after one, would have
// produced a status line here in the middle of the other half's output. So
// an unrecognised character is now silence, and a host that wants status
// asks for it.
//
// A dump is 2066 characters, which at 115200 baud takes about 180 ms. If a
// frame completes while a dump is still going out the new frame is
// **counted in `dropped` and not sent**, because a half-finished dump
// interleaved with the next one would be worse than a gap. A host that
// wants every frame must either slow the driver down or raise the baud
// rate.
//
// ===================================================================
// THE STATUS LINE
// ===================================================================
//
//   [127:112]  frames completed
//   [111: 96]  data bytes written to GDDRAM
//   [ 95: 80]  command bytes seen
//   [ 79: 64]  commands this design does not implement
//   [ 63: 48]  bit errors from the SPI receiver   should be 0
//   [ 47: 32]  frames dropped because a dump was in progress
//   [ 31: 24]  flags: 7 display_on, 6 inverse, 5 all_on, 4 charge_pump,
//              3 seg_remap, 2 com_reverse, 1 dumping, 0 always set
//   [ 23: 16]  the column the next data byte goes to
//   [ 15:  8]  page in [7:4], addressing mode in [3:2], pending
//              arguments in [1:0]
//   [  7:  0]  a sequence number, which wraps
//
// Bit 0 of the flags is always set for the same reason `selftest.v`
// carries a fixed `A5C3`: so that a line of all zeros cannot be mistaken
// for a working link reporting an idle screen.
//
// ===================================================================
// READING A DUMP
// ===================================================================
//
// Each line is one page: 128 bytes, 256 hex characters, most significant
// nibble first. **A byte is eight vertically stacked pixels**, bit 0 the
// topmost — so the pixel at (x, y) is bit `y % 8` of byte `x` of line
// `y / 8`. That is an SSD1306's own layout and nothing here transposes it,
// because a scanline order is a format no SSD1306 driver speaks.
//
// Rendering 128x64 from eight such lines is three lines of host code and
// no RTL, which is the right side of that trade.
//
// ===================================================================
// WHAT THE BOARD SHOWS WITHOUT A HOST
// ===================================================================
//
//   LD0..LD7   the last byte written to the display
//   LD8        the display has been told it is on
//   LD9        a dump is going out
//   LD10       dumping after every frame, from `o`
//   LD11       the SPI receiver has seen a bit error    should stay dark
//   LD12       a command arrived that this design does not implement
//   LD13       a frame was dropped
//   LD14       a heartbeat
//   digits     frames completed, in hexadecimal
//
// LD6 is skipped: ball U14 sits in a `LIOB33_SING` tile, so `led[5]` is
// LD5 and `led[6]` is LD7. That was a gap in the flow and never a dead
// ball, and the gap is closed — `fpga::xray::TileAlias` reaches that
// tile type and `examples/basys3/io_exercise.v` drives all sixteen. The
// numbering here is kept because the LED map above is what a person
// reads off the board.
//
// ===================================================================
// WHAT IS CHECKED
// ===================================================================
//
// `ssd1306_console_tb.v` drives real SSD1306 traffic into the four pins —
// an initialisation sequence and a full frame, bit by bit, with chip
// select and D/C as a real driver would — then asks over the serial port
// and checks all 1024 bytes of the dump against what it drew.
//
// **Both halves have run on a Basys 3.** The SPI half did first, inside
// `iso_display.v` (`78a162f`): a real device drew its logo into this core
// and the dump rendered it legibly, with `unknown` at zero over 3072 bytes.
// This standalone wrapper was then CHECKED on 9 October 2026 with nothing
// on the SPI pins: `?` gave 300 consecutive correct status lines, `g` a
// correct empty frame, and `z`, `o`, `n` and unknown characters were silent
// as documented. That needed a fix in the flow first: the status line had
// come back with bits taken from the next nibble, a hold violation from a
// clock routed off the global network (`docs/fpga-xray.md`, "A clock off
// the network") — and `iso_display.v` has not been rebuilt and run with
// that fix yet. `<`/`>` have not been tried from this wrapper.
//
// **This wrapper's port is 115200 baud; `iso_display.v`'s is 2 Mbaud.**
//
// `ip/bus/spi_display_rx`'s `frame_count` is known to read one too high
// from reset (its README §3a). The SPI receiver's frame counter is not in
// the status line for that reason; the one reported is the **display's**,
// which counts completed frames and is checked.

module ssd1306_console #(
    parameter TICK_BIT = 25,
    parameter COLUMNS  = 128,
    parameter PAGES    = 8,
    // How long a button is held, as a power of two system clocks. 23 is
    // 75 ms at 112 MHz and 84 ms at 100 — comfortably longer than any
    // debounce a device is likely to apply, and short enough not to look
    // like a long press.
    parameter PRESS_BITS = 23,
    // How many clocks a press lasts; it must fit in PRESS_BITS + 1 bits.
    // The default is the old `2**PRESS_BITS`. `iso_display_pad.v` asks for
    // 100 ms, a human press.
    parameter integer PRESS_COUNT = 1 << PRESS_BITS,
    // Passed straight to `ssd1306_slave`: 1 puts the 1024-byte frame
    // buffer in a block RAM, which is right, and 0 puts it in lookup
    // tables. It is a parameter here only so a build can be tried both
    // ways from the top; see that module's comment on the memory.
    parameter BLOCK_RAM = 1
) (
    input  wire        clk,
    // Active low, synchronous, from whoever owns the clock: a power-on
    // reset on the board, and the same net the serial port resets on so
    // that neither comes up while the other is still held.
    input  wire        rst_n,

    // Pmod JA, pins 1 to 4.
    input  wire        sclk,
    input  wire        mosi,
    input  wire        dc,
    input  wire        cs_n,

    // **Bytes, not pins**: `cmd_valid` is the one-cycle strobe
    // `ip/bus/uart`'s receiver gives, and `out_valid` is held with
    // `out_data` stable until a cycle in which `out_ready` is high too.
    input  wire        cmd_valid,
    input  wire [7:0]  cmd_data,
    output wire        out_valid,
    output wire [7:0]  out_data,
    input  wire        out_ready,

    // A request to press one of the far side's own button lines, high for
    // `2**PRESS_BITS` clocks. **Tristate is not here**: the line is
    // released except while pressed, which is what keeps the physical
    // buttons working and guarantees nothing is ever fought, and the
    // module that owns the ball is the one that releases it.
    output wire        press_left,
    output wire        press_right,

    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    localparam integer WORDS = COLUMNS * PAGES;

    reg [TICK_BIT:0] tick = {(TICK_BIT + 1){1'b0}};
    always @(posedge clk) tick <= tick + 1'b1;

    // =================================================================
    // The host's byte streams
    // =================================================================
    //
    // The emitter at the bottom was written against a `uart` instance's
    // `tx_data`/`tx_valid`/`tx_ready` and still is: the port moved out, the
    // handshake did not change.
    reg  [7:0] tx_data  = 8'd0;
    reg        tx_valid = 1'b0;

    assign out_data  = tx_data;
    assign out_valid = tx_valid;
    wire   tx_ready  = out_ready;

    // `z` holds both blocks in reset for sixteen cycles, which clears
    // every counter and the display's state.
    reg [4:0] clear = 5'd0;
    always @(posedge clk) begin
        if (cmd_valid && cmd_data == 8'h7A)  clear <= 5'd16;  // 'z'
        else if (clear != 5'd0)              clear <= clear - 5'd1;
    end
    wire sub_rst_n = rst_n & (clear == 5'd0);

    // =================================================================
    // The wire, and the display
    // =================================================================

    wire [7:0]  rx_byte;
    wire        rx_is_data, rx_valid;
    wire [15:0] spi_bit_errors;

    spi_display_rx #(.COUNT_WIDTH(16)) wire_rx (
        .clk(clk), .rst_n(sub_rst_n),
        .sclk(sclk), .mosi(mosi), .dc(dc), .cs_n(cs_n),
        .rx_byte(rx_byte), .rx_is_data(rx_is_data), .rx_valid(rx_valid),
        .cmd_byte_count(), .data_byte_count(), .frame_count(),
        .bit_error_count(spi_bit_errors), .dc_change_count(),
        .overrun_count(), .bit_count(), .last_bit_count(),
        .framed(), .framing_error(), .dc_error(), .overrun());

    reg  [15:0] rd_addr = 16'd0;
    wire [7:0]  rd_data;
    wire        frame_done;
    wire        display_on, inverse, all_on, charge_pump, seg_remap,
                com_reverse;
    wire [7:0]  contrast, col_ptr, page_ptr;
    wire [1:0]  addr_mode, pending_args;
    wire [15:0] ssd_cmds, ssd_datas, ssd_frames, ssd_unknown;

    ssd1306_slave #(
        .COLUMNS(COLUMNS), .PAGES(PAGES), .COUNT_WIDTH(16),
        .BLOCK_RAM(BLOCK_RAM)
    ) panel (
        .clk(clk), .rst_n(sub_rst_n),
        .in_byte(rx_byte), .in_is_data(rx_is_data), .in_valid(rx_valid),
        .rd_addr(rd_addr), .rd_data(rd_data),
        .frame_done(frame_done),
        .display_on(display_on), .inverse(inverse), .all_on(all_on),
        .contrast(contrast), .addr_mode(addr_mode),
        .charge_pump(charge_pump), .seg_remap(seg_remap),
        .com_reverse(com_reverse),
        .col_ptr(col_ptr), .page_ptr(page_ptr),
        .pending_args(pending_args),
        .cmd_count(ssd_cmds), .data_count(ssd_datas),
        .frame_count(ssd_frames), .unknown_cmds(ssd_unknown));

    // The last byte written, for the LEDs.
    reg [7:0] last_byte = 8'd0;
    always @(posedge clk) begin
        if (!sub_rst_n)                  last_byte <= 8'd0;
        else if (rx_valid && rx_is_data) last_byte <= rx_byte;
    end

    // =================================================================
    // What to send
    // =================================================================

    reg        auto    = 1'b0;
    reg        dumping = 1'b0;
    reg        sending = 1'b0;
    reg [7:0]  seq     = 8'd0;
    reg [15:0] dropped = 16'd0;

    always @(posedge clk) begin
        if (!rst_n) auto <= 1'b0;
        else if (cmd_valid) begin
            if (cmd_data == 8'h6F)      auto <= 1'b1;   // 'o'
            else if (cmd_data == 8'h6E) auto <= 1'b0;   // 'n'
        end
    end

    // ---- The two button lines ----
    //
    // A timed press rather than a toggle: the host asks once and the
    // hardware holds the line for `2**PRESS_BITS` clocks, so a busy host
    // cannot accidentally leave a button down or release it too soon for
    // the far side's debounce to see it.
    localparam [PRESS_BITS:0] PRESS_LOAD = PRESS_COUNT;
    reg [PRESS_BITS:0] left_hold  = {(PRESS_BITS + 1){1'b0}};
    reg [PRESS_BITS:0] right_hold = {(PRESS_BITS + 1){1'b0}};

    always @(posedge clk) begin
        if (!rst_n) begin
            left_hold  <= {(PRESS_BITS + 1){1'b0}};
            right_hold <= {(PRESS_BITS + 1){1'b0}};
        end else begin
            if (cmd_valid && cmd_data == 8'h3C)        // '<'
                left_hold <= PRESS_LOAD;
            else if (left_hold != 0)
                left_hold <= left_hold - 1'b1;

            if (cmd_valid && cmd_data == 8'h3E)        // '>'
                right_hold <= PRESS_LOAD;
            else if (right_hold != 0)
                right_hold <= right_hold - 1'b1;
        end
    end

    // Asked for here, released or driven one ball up.
    assign press_left  = (left_hold  != 0);
    assign press_right = (right_hold != 0);

    wire [7:0] flags = {display_on, inverse, all_on, charge_pump,
                        seg_remap, com_reverse, dumping, 1'b1};

    wire [127:0] status = {
        ssd_frames, ssd_datas, ssd_cmds, ssd_unknown,
        spi_bit_errors, dropped,
        flags, col_ptr,
        page_ptr[3:0], addr_mode, pending_args,
        seq
    };

    // **One character, not "everything else".** Sharing a serial port with
    // the card half means seeing its commands and the line endings a host
    // sends after them, and answering every one of those with a status line
    // would bury the card's own output. So status is asked for by name and
    // an unrecognised character is silence.
    wire want_status = cmd_valid && cmd_data == 8'h3F;   // '?'
    wire want_dump   = cmd_valid && cmd_data == 8'h67;   // 'g'

    // **A queue, not a flag.** Each request is pushed when it arrives and
    // popped when its own line *starts*, so one that arrives while an
    // earlier answer is still on the wire waits its turn instead of being
    // lost. Two flags did lose them: the status flag was cleared on every
    // cycle the port was sending, so on a Basys 3 at 2 Mbaud the second of
    // two back-to-back `?` got no answer at all, and a `g` during a dump
    // likewise. `ssd1306_console_tb.v` sends `?`, `?`, `g`, `?` back to
    // back and wants every answer.
    //
    // One bit per request, oldest in bit 0: 0 a status line, 1 a dump.
    // Sixteen deep. A host gets one line of at least 34 characters per
    // one-character request, so only one that sends more than sixteen
    // requests without reading can fill it; a seventeenth outstanding
    // request is dropped, since this port has no flow control to hold
    // the host off with.
    //
    // A dump asked for by `o` is not a request and does not queue: it keeps
    // its own flag, and a frame that completes while one is waiting or
    // going out is counted in `dropped`, as before.
    localparam integer QUEUE = 16;
    reg [QUEUE-1:0] queue        = {QUEUE{1'b0}};
    // Which entries are in use, as a thermometer: ones from bit 0 up. That
    // needs no adder, and the slot a new request lands in is the one bit a
    // push changes. A five-bit count was the first version, and its carry
    // chain was what the placer could not fit in `iso_display.v`.
    reg [QUEUE-1:0] held         = {QUEUE{1'b0}};
    reg             auto_pending = 1'b0;

    wire             take_auto = !sending && auto_pending;
    wire             pop       = !sending && !auto_pending && held[0];
    wire             push      = want_status || want_dump;
    wire [QUEUE-1:0] kept      = pop ? {1'b0, held[QUEUE-1:1]}  : held;
    wire [QUEUE-1:0] shifted   = pop ? {1'b0, queue[QUEUE-1:1]} : queue;
    wire             fits      = !kept[QUEUE-1];
    wire [QUEUE-1:0] grown     = {kept[QUEUE-2:0], 1'b1};
    wire [QUEUE-1:0] slot      = grown & ~kept;

    always @(posedge clk) begin
        if (!rst_n) begin
            queue        <= {QUEUE{1'b0}};
            held         <= {QUEUE{1'b0}};
            auto_pending <= 1'b0;
            dropped      <= 16'd0;
        end else begin
            queue <= (push && fits && want_dump) ? (shifted | slot) : shifted;
            held  <= (push && fits) ? grown : kept;

            if (take_auto) auto_pending <= 1'b0;
            if (auto && frame_done) begin
                // A frame while a dump is going out is counted, not sent.
                if (dumping || (auto_pending && !take_auto)) begin
                    if (dropped != 16'hFFFF) dropped <= dropped + 16'd1;
                end else begin
                    auto_pending <= 1'b1;
                end
            end
        end
    end

    // =================================================================
    // The emitter
    // =================================================================
    //
    // One handshake for both kinds of output: a character is presented and
    // held until `tx_ready` is high in the same cycle. Pulsing `tx_valid`
    // for one cycle instead would be a race, since the cycle it appears in
    // is not the cycle its `tx_ready` was read in.

    localparam [1:0] SRC_NONE = 2'd0, SRC_STATUS = 2'd1, SRC_DUMP = 2'd2;

    reg [1:0]   source  = SRC_NONE;
    reg [127:0] shifter = 128'd0;
    reg [5:0]   left    = 6'd0;
    reg [1:0]   tail    = 2'd0;

    // The dump's own position: which byte, and which nibble of it.
    reg [15:0] dump_addr = 16'd0;
    reg [7:0]  dump_byte = 8'd0;
    reg [1:0]  dump_step = 2'd0;   // 0 fetch, 1 high nibble, 2 low nibble

    function [7:0] hex;
        input [3:0] nibble;
        hex = (nibble < 4'd10) ? (8'd48 + {4'd0, nibble})
                               : (8'd55 + {4'd0, nibble});
    endfunction

    // **Two cycles of read latency, not one.** `rd_addr` is a register and
    // so is `rd_data`, so a byte asked for at the end of cycle N is only
    // readable in cycle N+2. The first version of this waited one cycle,
    // latched the previous byte, and emitted byte 0 twice while losing the
    // last one — `ssd1306_console_tb.v` caught it as "page 0 column 1
    // dumped 07, drew 12", which is byte zero's value in byte one's place.
    // Hence a fetch of two steps: 0 issues the wait, 3 takes the byte.
    always @(posedge clk) begin
        if (!rst_n) begin
            sending   <= 1'b0;
            dumping   <= 1'b0;
            tx_valid  <= 1'b0;
            source    <= SRC_NONE;
            dump_addr <= 16'd0;
            dump_step <= 2'd0;
            rd_addr   <= 16'd0;
        end else if (!sending) begin
            if (take_auto || (pop && queue[0])) begin
                sending   <= 1'b1;
                dumping   <= 1'b1;
                source    <= SRC_DUMP;
                dump_addr <= 16'd0;
                dump_step <= 2'd0;
                rd_addr   <= 16'd0;
                tail      <= 2'd0;
            end else if (pop) begin
                sending <= 1'b1;
                source  <= SRC_STATUS;
                shifter <= status;
                left    <= 6'd32;
                tail    <= 2'd2;
            end
        end else if (source == SRC_STATUS) begin
            if (!tx_valid) begin
                if (left != 6'd0)      tx_data <= hex(shifter[127:124]);
                else if (tail == 2'd2) tx_data <= 8'd13;
                else                   tx_data <= 8'd10;
                tx_valid <= 1'b1;
            end else if (tx_ready) begin
                tx_valid <= 1'b0;
                if (left != 6'd0) begin
                    shifter <= {shifter[123:0], 4'd0};
                    left    <= left - 6'd1;
                end else if (tail == 2'd2) begin
                    tail <= 2'd1;
                end else begin
                    tail    <= 2'd0;
                    sending <= 1'b0;
                    source  <= SRC_NONE;
                    seq     <= seq + 8'd1;
                end
            end
        end else begin
            // SRC_DUMP
            if (dump_step == 2'd0) begin
                // The address was set last cycle; `rd_data` is not ready
                // until the next one.
                dump_step <= 2'd3;
            end else if (dump_step == 2'd3) begin
                dump_byte <= rd_data;
                dump_step <= 2'd1;
            end else if (tail != 2'd0) begin
                // End of a page's line: CR then LF.
                if (!tx_valid) begin
                    tx_data  <= (tail == 2'd2) ? 8'd13 : 8'd10;
                    tx_valid <= 1'b1;
                end else if (tx_ready) begin
                    tx_valid <= 1'b0;
                    if (tail == 2'd2) begin
                        tail <= 2'd1;
                    end else begin
                        tail <= 2'd0;
                        if (dump_addr == WORDS) begin
                            sending <= 1'b0;
                            dumping <= 1'b0;
                            source  <= SRC_NONE;
                            seq     <= seq + 8'd1;
                        end else begin
                            rd_addr   <= dump_addr;
                            dump_step <= 2'd0;
                        end
                    end
                end
            end else if (!tx_valid) begin
                tx_data  <= (dump_step == 2'd1) ? hex(dump_byte[7:4])
                                                : hex(dump_byte[3:0]);
                tx_valid <= 1'b1;
            end else if (tx_ready) begin
                tx_valid <= 1'b0;
                if (dump_step == 2'd1) begin
                    dump_step <= 2'd2;
                end else begin
                    dump_addr <= dump_addr + 16'd1;
                    // A line per page.
                    if (((dump_addr + 16'd1) % COLUMNS) == 16'd0) begin
                        tail <= 2'd2;
                    end else begin
                        rd_addr   <= dump_addr + 16'd1;
                        dump_step <= 2'd0;
                    end
                end
            end
        end
    end

    // =================================================================
    // The board's own display
    // =================================================================

    assign led = {tick[TICK_BIT],
                  dropped != 16'd0,
                  ssd_unknown != 16'd0,
                  spi_bit_errors != 16'd0,
                  auto, dumping, display_on,
                  last_byte};

    reg [17:0] mux = 18'd0;
    always @(posedge clk) mux <= mux + 18'd1;
    wire [1:0] digit = mux[17:16];

    wire [3:0] nibble =
        (digit == 2'd0) ? ssd_frames[3:0]   :
        (digit == 2'd1) ? ssd_frames[7:4]   :
        (digit == 2'd2) ? ssd_frames[11:8]  :
                          ssd_frames[15:12];

    // Active low, and **the bit order is `gfedcba`**: the leftmost bit of
    // a `7'b...` literal is the most significant, which is `seg[6]`, which
    // is segment g. The first version of this table was written as though
    // the leftmost bit were segment a, so every pattern was bit-reversed —
    // a `0` came out as a zero with no top bar and a lit middle bar.
    //
    // Nothing in simulation can catch that: the patterns are arbitrary
    // constants and a reversed font is as self-consistent as a correct
    // one. Only a person looking at the glass can, and one did.
    //
    // Checked against the standard active-low hex font: C0 F9 A4 B0 99 92
    // 82 F8 80 90 88 83 C6 A1 86 8E.
    function [6:0] segments;
        input [3:0] value;
        case (value)
            4'h0: segments = 7'b1000000;
            4'h1: segments = 7'b1111001;
            4'h2: segments = 7'b0100100;
            4'h3: segments = 7'b0110000;
            4'h4: segments = 7'b0011001;
            4'h5: segments = 7'b0010010;
            4'h6: segments = 7'b0000010;
            4'h7: segments = 7'b1111000;
            4'h8: segments = 7'b0000000;
            4'h9: segments = 7'b0010000;
            4'hA: segments = 7'b0001000;
            4'hB: segments = 7'b0000011;
            4'hC: segments = 7'b1000110;
            4'hD: segments = 7'b0100001;
            4'hE: segments = 7'b0000110;
            default: segments = 7'b0001110;  // F
        endcase
    endfunction

    assign seg = segments(nibble);
    // The point blinks as a heartbeat and lights while either button is
    // pressed, so a press is visible on the board itself.
    assign dp  = ~(tick[TICK_BIT] | press_left | press_right);
    assign an  = ~(4'd1 << digit);
endmodule
