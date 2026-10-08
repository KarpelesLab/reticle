// An SPI slave on a Digilent Basys 3's Pmod header, driven and read over
// the board's serial port.
//
// This is the first design in this directory meant for use rather than
// for proving the backend: plug a display's four-wire SPI into Pmod JA,
// open the serial port at 115200, and ask the part what it has seen.
// `ip/bus/spi_display_rx` does the receiving — oversampled, no edge of
// `sclk` clocking anything — and `ip/bus/uart` carries the conversation.
//
// ===================================================================
// WIRING
// ===================================================================
//
// Pmod JA, the four pins of its top row, numbered as Digilent prints
// them on the board:
//
//   JA1  J1   sclk    the far side's clock
//   JA2  L2   mosi    its data, command and pixel alike
//   JA3  J2   dc      low for a command byte, high for a pixel byte
//   JA4  G2   cs_n    low while a byte is on the wire
//
// JA5 is ground and JA6 is 3V3 on this header, so a four-wire link needs
// no other connection. **The rate limit is a ratio, not a frequency**:
// `PHASE_MARGIN = 2` means each `sclk` phase must last at least two
// 100 MHz cycles, so `sclk` may run up to 25 MHz. `overrun_count` rises
// before any bit is lost, which is what makes it worth reading.
//
// Nothing here drives any of the four pins. It is a receiver.
//
// ===================================================================
// THE CONVERSATION
// ===================================================================
//
// Send one character; get one line of 32 hexadecimal characters and a
// CRLF. Unknown characters report, which means a stray newline costs a
// line of output and never silence.
//
//   c   report (any unrecognised character does the same)
//   r   reset the receiver and its counters, then report
//   a   start reporting about three times a second, unprompted
//   q   stop reporting unprompted
//
// The line, most significant character first:
//
//   [127:112]  command bytes seen       `dc` low
//   [111: 96]  data bytes seen          `dc` high
//   [ 95: 80]  frames seen
//   [ 79: 64]  bit errors               a frame that was not eight bits
//   [ 63: 48]  `dc` changes mid-frame
//   [ 47: 32]  overruns                 a byte arrived before the last
//                                       was taken
//   [ 31: 24]  the last byte received
//   [ 23: 16]  flags: bit 7 `rx_is_data`, 6 `framed`, 5 `framing_error`,
//              4 `dc_error`, 3 `overrun`, and bit 0 set so that a line
//              of all zeros can never be mistaken for a working link
//   [ 15: 12]  `bit_count`              bits in the frame now arriving
//   [ 11:  8]  `last_bit_count`         bits the previous frame carried
//   [  7:  0]  a sequence number, which wraps
//
// **Which numbers should be zero**: bit errors, `dc` changes and
// overruns. `ip/bus/spi_display_rx/README.md` §4 is the counter-by-
// counter bring-up table and §6 the order to suspect things in. The
// quickest read of a live link is that command and data byte counts both
// advance and the other three stay at zero.
//
// ===================================================================
// WHAT THE BOARD SHOWS WITHOUT A HOST
// ===================================================================
//
//   LD0..LD7   the last byte received, LD0 its lowest bit
//   LD8        `rx_is_data`: the last byte was a pixel, not a command
//   LD9        `framed`
//   LD10       `framing_error`
//   LD11       `dc_error`
//   LD12       `overrun`
//   LD13       reporting unprompted, from `a`
//   LD15       a heartbeat, about three times every two seconds
//   digits     frames seen, in hexadecimal
//   point      blinks with the heartbeat
//
// LD14 is where LD6 would be in a sixteen-LED design: ball U14 sits in a
// `LIOB33_SING` tile this flow had no IO table for when this design was
// written, so it is skipped and the design has fifteen LEDs. That was a
// gap in the flow, not a dead ball — the board's owner confirms LD6
// works — and the gap is closed: `fpga::xray::TileAlias` reaches that
// tile type and `examples/basys3/io_exercise.v` drives all sixteen.
//
// ===================================================================
// WHAT IS AND IS NOT CHECKED HERE
// ===================================================================
//
// On the part, by `docs/fpga-xray.md`'s account: the carry chain, a
// `RAMB18E1` with contents, four `RAM64X1D`, a `PLLE2_BASE`'s lock, the
// serial port in both directions, and seven output pads. `spi_console.v`
// uses the carry chain and the serial port and nothing else from that
// list.
//
// **Not checked: the Pmod pins.** No signal has been driven into JA on
// this board. `examples/basys3/io_exercise.v` is how to check a pad
// before trusting it, and the counters above are how to check the link
// once something is plugged in — two zeros where zeros belong are worth
// more than this comment.

module spi_console #(
    // 100 MHz / 115200 = 868.
    parameter CLK_DIV = 868,
    // Which bit of a free-running counter paces an unprompted report.
    parameter TICK_BIT = 25
) (
    input  wire        clk,

    // Pmod JA, pins 1 to 4.
    input  wire        sclk,
    input  wire        mosi,
    input  wire        dc,
    input  wire        cs_n,

    // The board's USB-UART bridge.
    input  wire        uart_rx_pin,
    output wire        uart_tx_pin,

    output wire [14:0] led,
    output wire [6:0]  seg,
    output wire        dp,
    output wire [3:0]  an
);
    // ---- Reset, saturating so it can never un-reset. ----
    //
    // A shift register filling with ones, not a one-hot walking under an
    // enable: that walks off the end and disarms itself, which is a
    // mistake this directory has already made once on hardware.
    reg [15:0] por = 16'h0000;
    always @(posedge clk) por <= {por[14:0], 1'b1};
    wire rst_n = por[15];

    reg [TICK_BIT:0] tick = {(TICK_BIT + 1){1'b0}};
    reg              tick_d = 1'b0;
    always @(posedge clk) begin
        tick   <= tick + 1'b1;
        tick_d <= tick[TICK_BIT];
    end
    wire tick_edge = tick[TICK_BIT] & ~tick_d;

    // =================================================================
    // The serial port
    // =================================================================

    wire [7:0] cmd_data;
    wire       cmd_valid;
    wire       tx_ready;
    reg  [7:0] tx_data  = 8'd0;
    reg        tx_valid = 1'b0;

    uart #(
        .CLK_DIV(CLK_DIV)
    ) port (
        .clk             (clk),
        .rst_n           (rst_n),
        .div             (16'd0),
        .tx_data         (tx_data),
        .tx_valid        (tx_valid),
        .tx_ready        (tx_ready),
        .tx              (uart_tx_pin),
        .rx              (uart_rx_pin),
        .rx_data         (cmd_data),
        .rx_valid        (cmd_valid),
        .rx_error        (),
        .rx_frame_error  (),
        .rx_parity_error (),
        .rx_break        ()
    );

    // =================================================================
    // The receiver
    // =================================================================

    // `r` holds it in reset for sixteen cycles, which clears every
    // counter: the block has no separate clear and does not need one.
    reg [4:0] clear = 5'd0;
    always @(posedge clk) begin
        if (cmd_valid && cmd_data == 8'h72)      // 'r'
            clear <= 5'd16;
        else if (clear != 5'd0)
            clear <= clear - 5'd1;
    end
    wire spi_rst_n = rst_n & (clear == 5'd0);

    wire [7:0]  rx_byte;
    wire        rx_is_data, rx_valid;
    wire [15:0] cmd_bytes, data_bytes, frames, bit_errors, dc_changes,
                overruns;
    wire [3:0]  bit_count, last_bit_count;
    wire        framed, framing_error, dc_error, overrun;

    spi_display_rx #(
        .COUNT_WIDTH(16)
    ) rx (
        .clk             (clk),
        .rst_n           (spi_rst_n),
        .sclk            (sclk),
        .mosi            (mosi),
        .dc              (dc),
        .cs_n            (cs_n),
        .rx_byte         (rx_byte),
        .rx_is_data      (rx_is_data),
        .rx_valid        (rx_valid),
        .cmd_byte_count  (cmd_bytes),
        .data_byte_count (data_bytes),
        .frame_count     (frames),
        .bit_error_count (bit_errors),
        .dc_change_count (dc_changes),
        .overrun_count   (overruns),
        .bit_count       (bit_count),
        .last_bit_count  (last_bit_count),
        .framed          (framed),
        .framing_error   (framing_error),
        .dc_error        (dc_error),
        .overrun         (overrun)
    );

    // The last byte, held so that a host asking later still sees it.
    reg [7:0] held      = 8'd0;
    reg       held_data = 1'b0;
    always @(posedge clk) begin
        if (!spi_rst_n) begin
            held      <= 8'd0;
            held_data <= 1'b0;
        end else if (rx_valid) begin
            held      <= rx_byte;
            held_data <= rx_is_data;
        end
    end

    // =================================================================
    // What to say, and when
    // =================================================================

    // Declared here rather than beside the emitter below, because
    // `report` reads `seq` and `pending` reads `sending`, and a name must
    // be declared before it is used.
    reg [127:0] shifter = 128'd0;
    reg [5:0]   left    = 6'd0;
    reg [1:0]   tail    = 2'd0;
    reg         sending = 1'b0;
    reg [7:0]   seq     = 8'd0;

    reg auto = 1'b0;
    always @(posedge clk) begin
        if (cmd_valid) begin
            if (cmd_data == 8'h61) auto <= 1'b1;        // 'a'
            else if (cmd_data == 8'h71) auto <= 1'b0;   // 'q'
        end
    end

    // Bit 0 of the flags is always set, so a line of all zeros cannot be
    // mistaken for a quiet but working link — the same reason
    // `selftest.v` carries a fixed `A5C3`.
    wire [7:0] flags = {held_data, framed, framing_error, dc_error,
                        overrun, 2'b00, 1'b1};

    wire [127:0] report = {
        cmd_bytes,                  // [127:112]
        data_bytes,                 // [111: 96]
        frames,                     // [ 95: 80]
        bit_errors,                 // [ 79: 64]
        dc_changes,                 // [ 63: 48]
        overruns,                   // [ 47: 32]
        held,                       // [ 31: 24]
        flags,                      // [ 23: 16]
        bit_count, last_bit_count,  // [ 15:  8]
        seq                         // [  7:  0]
    };

    // `q` is the one command that does not report: asking for silence and
    // being answered would be a poor joke.
    wire asked = cmd_valid && cmd_data != 8'h71;
    reg  pending = 1'b0;
    always @(posedge clk) begin
        if (asked || (auto && tick_edge)) pending <= 1'b1;
        else if (sending)                 pending <= 1'b0;
    end

    // =================================================================
    // Sending it
    // =================================================================

    function [7:0] hex;
        input [3:0] nibble;
        hex = (nibble < 4'd10) ? (8'd48 + {4'd0, nibble})
                               : (8'd55 + {4'd0, nibble});
    endfunction

    always @(posedge clk) begin
        if (!rst_n) begin
            sending  <= 1'b0;
            tx_valid <= 1'b0;
        end else if (!sending) begin
            if (pending) begin
                shifter <= report;
                left    <= 6'd32;
                tail    <= 2'd2;
                sending <= 1'b1;
            end
        end else if (!tx_valid) begin
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
                seq     <= seq + 8'd1;
            end
        end
    end

    // =================================================================
    // The board's own display
    // =================================================================

    assign led = {tick[TICK_BIT], auto, overrun, dc_error, framing_error,
                  framed, held_data, held};

    reg [17:0] mux = 18'd0;
    always @(posedge clk) mux <= mux + 18'd1;
    wire [1:0] digit = mux[17:16];

    wire [3:0] nibble =
        (digit == 2'd0) ? frames[3:0]   :
        (digit == 2'd1) ? frames[7:4]   :
        (digit == 2'd2) ? frames[11:8]  :
                          frames[15:12];

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
    assign dp  = ~tick[TICK_BIT];
    assign an  = ~(4'd1 << digit);
endmodule
