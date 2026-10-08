// A Digilent Basys 3 pretending to be a 128x64 SSD1306, with the pixels
// readable over the board's serial port.
//
// Three blocks and a serial port:
//
//   ip/bus/spi_display_rx    the four wires -> bytes with a D/C tag
//   ip/video/ssd1306_slave   those bytes -> the command set and GDDRAM
//   ip/bus/uart              a host asks, this answers
//
// Plug a driver's SPI output into Pmod JA, open the serial port at 115200,
// and read the screen. `frame_done` from the slave says when a whole frame
// has landed, so `a` streams a copy after every redraw without a host
// having to guess at timing.
//
// ===================================================================
// THE CONVERSATION
// ===================================================================
//
//   s   status: one line of 32 hex characters (below)
//   d   dump the frame buffer: **8 lines of 256 hex characters**, one
//       line per page, 1024 bytes in all
//   a   dump after every completed frame, unprompted
//   q   stop dumping unprompted
//   r   reset the receiver and the display, then report status
//   L   press the far side's LEFT button for `2**PRESS_BITS` clocks
//   R   the same for its RIGHT button
//
// Anything else reports status, so a stray newline costs a line and never
// silence.
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
//   LD10       dumping after every frame, from `a`
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
// **No part has run this**, and `ip/bus/spi_display_rx`'s `frame_count` is
// known to read one too high from reset (its README §3a). The SPI
// receiver's frame counter is not in the status line for that reason; the
// one reported is the **display's**, which counts completed frames and is
// checked.

module ssd1306_console #(
    parameter CLK_DIV  = 868,   // 100 MHz / 115200
    parameter TICK_BIT = 25,
    parameter COLUMNS  = 128,
    parameter PAGES    = 8,
    // How long a button is held, as a power of two system clocks. 23 is
    // 75 ms at 112 MHz and 84 ms at 100 — comfortably longer than any
    // debounce a device is likely to apply, and short enough not to look
    // like a long press.
    parameter PRESS_BITS = 23
) (
    input  wire        clk,

    // Pmod JA, pins 1 to 4.
    input  wire        sclk,
    input  wire        mosi,
    input  wire        dc,
    input  wire        cs_n,

    input  wire        uart_rx_pin,
    output wire        uart_tx_pin,

    // Two of the far side's own button lines, pressed electrically.
    //
    // **Both are released except while pressed**, which is what makes the
    // physical buttons still work and guarantees nothing is ever fought.
    // Each line has its own 10k resistor defining its idle level, so a
    // press means overpowering that resistor in one direction only — and
    // the two directions differ:
    //
    //   `btn_left`   idles LOW through a pull-down  -> press drives HIGH
    //   `btn_right`  idles HIGH through a pull-up   -> press drives LOW
    //
    // A push-pull output would hold each line at its idle level and fight
    // anybody pressing the real button, which with a switch to the
    // opposite rail is a short limited only by the pin.
    inout  wire        btn_left,
    inout  wire        btn_right,

    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    localparam integer WORDS = COLUMNS * PAGES;

    // Saturating reset: ones shift in, so it cannot un-reset. A one-hot
    // walking under an enable can walk off the end, which this directory
    // has already paid for once on hardware.
    reg [15:0] por = 16'h0000;
    always @(posedge clk) por <= {por[14:0], 1'b1};
    wire rst_n = por[15];

    reg [TICK_BIT:0] tick = {(TICK_BIT + 1){1'b0}};
    always @(posedge clk) tick <= tick + 1'b1;

    // =================================================================
    // The serial port
    // =================================================================

    wire [7:0] cmd_data;
    wire       cmd_valid;
    wire       tx_ready;
    reg  [7:0] tx_data  = 8'd0;
    reg        tx_valid = 1'b0;

    uart #(.CLK_DIV(CLK_DIV)) port (
        .clk(clk), .rst_n(rst_n), .div(16'd0),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
        .tx(uart_tx_pin),
        .rx(uart_rx_pin), .rx_data(cmd_data), .rx_valid(cmd_valid),
        .rx_error(), .rx_frame_error(), .rx_parity_error(), .rx_break());

    // `r` holds both blocks in reset for sixteen cycles, which clears
    // every counter and the display's state.
    reg [4:0] clear = 5'd0;
    always @(posedge clk) begin
        if (cmd_valid && cmd_data == 8'h72)  clear <= 5'd16;  // 'r'
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
        .COLUMNS(COLUMNS), .PAGES(PAGES), .COUNT_WIDTH(16)
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
            if (cmd_data == 8'h61)      auto <= 1'b1;   // 'a'
            else if (cmd_data == 8'h71) auto <= 1'b0;   // 'q'
        end
    end

    // ---- The two button lines ----
    //
    // A timed press rather than a toggle: the host asks once and the
    // hardware holds the line for `2**PRESS_BITS` clocks, so a busy host
    // cannot accidentally leave a button down or release it too soon for
    // the far side's debounce to see it.
    reg [PRESS_BITS:0] left_hold  = {(PRESS_BITS + 1){1'b0}};
    reg [PRESS_BITS:0] right_hold = {(PRESS_BITS + 1){1'b0}};

    always @(posedge clk) begin
        if (!rst_n) begin
            left_hold  <= {(PRESS_BITS + 1){1'b0}};
            right_hold <= {(PRESS_BITS + 1){1'b0}};
        end else begin
            if (cmd_valid && cmd_data == 8'h4C)        // 'L'
                left_hold <= {1'b1, {PRESS_BITS{1'b0}}};
            else if (left_hold != 0)
                left_hold <= left_hold - 1'b1;

            if (cmd_valid && cmd_data == 8'h52)        // 'R'
                right_hold <= {1'b1, {PRESS_BITS{1'b0}}};
            else if (right_hold != 0)
                right_hold <= right_hold - 1'b1;
        end
    end

    wire press_left  = (left_hold  != 0);
    wire press_right = (right_hold != 0);

    // Driven in one direction or released. Never driven to the idle level.
    assign btn_left  = press_left  ? 1'b1 : 1'bz;
    assign btn_right = press_right ? 1'b0 : 1'bz;

    wire [7:0] flags = {display_on, inverse, all_on, charge_pump,
                        seg_remap, com_reverse, dumping, 1'b1};

    wire [127:0] status = {
        ssd_frames, ssd_datas, ssd_cmds, ssd_unknown,
        spi_bit_errors, dropped,
        flags, col_ptr,
        page_ptr[3:0], addr_mode, pending_args,
        seq
    };

    // A status request: any character except `q`, which asks for silence,
    // and except `a`, which asks for frames and gets them.
    wire want_status = cmd_valid && cmd_data != 8'h71 && cmd_data != 8'h61
                                 && cmd_data != 8'h64 && cmd_data != 8'h4C
                                 && cmd_data != 8'h52;
    wire want_dump   = cmd_valid && cmd_data == 8'h64;   // 'd'

    reg status_pending = 1'b0;
    reg dump_pending   = 1'b0;

    always @(posedge clk) begin
        if (!rst_n) begin
            status_pending <= 1'b0;
            dump_pending   <= 1'b0;
            dropped        <= 16'd0;
        end else begin
            if (want_status)                      status_pending <= 1'b1;
            else if (sending && !dumping)         status_pending <= 1'b0;

            if (want_dump)                        dump_pending <= 1'b1;
            else if (auto && frame_done) begin
                // A frame while a dump is going out is counted, not sent.
                if (dumping || dump_pending) begin
                    if (dropped != 16'hFFFF) dropped <= dropped + 16'd1;
                end else begin
                    dump_pending <= 1'b1;
                end
            end else if (dumping)                 dump_pending <= 1'b0;
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
            if (dump_pending) begin
                sending   <= 1'b1;
                dumping   <= 1'b1;
                source    <= SRC_DUMP;
                dump_addr <= 16'd0;
                dump_step <= 2'd0;
                rd_addr   <= 16'd0;
                tail      <= 2'd0;
            end else if (status_pending) begin
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
