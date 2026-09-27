// A USB serial port on a Cynthion's AUX port, bridged to a UART:
// `ip/usb_cdc_acm` wired to the auxiliary ULPI transceiver, its bytes handed
// to `ip/uart`'s transmitter, and that transmitter's line **looped back into
// its own receiver** so that what a program writes to `/dev/ttyACM*` it reads
// back — having been serialised at 115200 baud in between.
//
// This is `usb_ulpi_device.v`'s successor and not its replacement. That design
// is a vendor-specific bulk loopback that no driver claims and it carries a
// hardware-verified constant-zero probe, so it stays exactly as it is; this
// one is the same board with a **class** on top, which is the thing a person
// can use without writing any host code at all.
//
// ===================================================================
// WHAT THERE IS TO LOOK AT, AND WHY IT IS NOT AN LED
// ===================================================================
//
// **The observable is a device file.** Either the kernel binds its own
// `cdc_acm` driver and a `/dev/ttyACM*` appears or it does not:
//
//     dmesg | tail
//     lsusb -d 1209:0001 -v
//     ls -l /dev/serial/by-id/
//
// and then, with no program of ours in the way at all:
//
//     stty -F /dev/ttyACM1 115200 raw -echo
//     printf 'hello\r\n' > /dev/ttyACM1 & cat -v /dev/ttyACM1
//
// `1209:0001` is pid.codes' test pair, the default of the core's `VID` and
// `PID`. **The number in `/dev/ttyACM*` is not fixed**: a Cynthion's own
// Apollo debugger is itself a CDC ACM device and is usually `ttyACM0`, so
// this one is normally `ttyACM1`. `/dev/serial/by-id/` names them.
//
// `tests/usb_cdc_acm.rs` is the same thing as a test, and it finds the port
// by walking sysfs for `1209:0001` rather than by guessing a number.
//
// The six LEDs are the **diagnosis** for when it does not appear, and the
// first four are `usb_ulpi_device.v`'s so that the two designs are read the
// same way:
//
//     LED 0   the transceiver is ready        `phy_ready`
//     LED 1   THE HOST CONFIGURED IT          `configured`, latched
//     LED 2   A BYTE WENT OUT TO THE UART     latched
//     LED 3   heartbeat, 0.89 Hz              the clock runs
//     LED 4   A BYTE CAME BACK FROM THE UART  latched
//     LED 5   THE UART'S TRANSMIT LINE        live, dark while idle
//
// Read from the bottom up, and `usb_ulpi_device.v`'s header has the whole
// ladder for LEDs 0, 1, 3 and the bus below them. What is new here:
//
//   * **LED 1 lit, LED 2 dark** — the host enumerated the device and no
//     program has written to the port. That is the resting state: opening
//     the port is not writing to it.
//   * **LED 2 lit, LED 4 dark** — a byte reached `uart_tx` and no byte ever
//     came back out of `uart_rx`. Since the two are wired together inside
//     the part, that is a baud divisor wrong enough to break framing, or a
//     receiver that never saw the start bit — and nothing on the board can
//     cause it.
//   * **LED 4 lit** — the whole path worked at least once: USB out, 8N1
//     down a wire, 8N1 back up, USB in.
//   * **LED 5** flickers while bytes flow and is dark otherwise, because the
//     line idles high and the LEDs are active low. It is also the one place
//     the serial data **leaves the part**: ball C11 carries the real 8N1
//     waveform, start bit, eight data bits and stop bit, at 115200 baud, so
//     an oscilloscope or a logic analyser on that LED sees the characters.
//
// ===================================================================
// THE LOOPBACK IS INSIDE THE DIE, AND THAT IS SAID PLAINLY
// ===================================================================
//
// `uart_rxd` is `uart_txd` and nothing else, so the byte the receiver
// recovers travelled along a net and not along a wire between two headers.
// What that does and does not establish:
//
//   * it **does** establish the whole chain a serial bridge is made of — the
//     class layer, the bulk endpoints, the byte interface, the transmitter's
//     framing, the receiver's start-bit search and mid-bit sampling, the
//     FIFO, and the return path — because every one of those is between the
//     two ends of the round trip;
//   * it **does not** establish anything about a cable, a level shifter or a
//     second device's idea of the baud rate.
//
// A pin was not used for the return leg on purpose. The only balls this
// board has free for user IO are the two PMOD headers, and
// `bidir_loopback.v` in this directory already wrote down why this project
// does not take them: nothing this machine can read says whether anything is
// plugged into a 2x6 header, and a pin whose net a schematic fully describes
// is a better bet than a pin that is probably unconnected. An LED's net is
// one resistor and one diode to a supply rail with the FPGA as its only
// driver, which is why LED 5 carries the waveform out and why nothing tries
// to read it back in.
//
// ===================================================================
// WHY DRIVING THESE PINS IS SAFE
// ===================================================================
//
// The same eight ULPI balls, the same clock and the same six LEDs
// `usb_ulpi_device.v` drives, for the same reasons and with the same
// argument; that file's header has the long form and `usb_cdc_uart.rcf`
// beside this one has every line's provenance. The short form: the data
// lines are released whenever `ulpi_dir` is high, which is the bus's own
// arbitration (ULPI 1.1 §3.3); every direction is the one Great Scott
// Gadgets' platform file gives it; the Type-C controllers, the VBUS switches
// and the pseudo-supply pins are left alone; and the `CONTROL` port the
// Apollo debugger lives on is a different transceiver on different balls and
// is not mentioned here, so loading this design cannot take the debugger
// away.
//
// ===================================================================
// THE BAUD DIVISOR, AND WHY IT IS A CONSTANT
// ===================================================================
//
// `CLK_DIV` is 521: 60 MHz over 115200 is 520.83, so the bit period is
// 521 cycles and the rate is 115163 baud — 0.032 % slow, against the 2 %
// `uart_tx`'s header says 8N1 survives.
//
// The host tells the device what rate it wants — `SET_LINE_CODING` carries
// `dwDTERate` and `ip/usb_cdc_acm` brings it out on `baud` — and this design
// **ignores it**, which is a choice and not an omission. Following it means
// dividing 60 000 000 by a run-time value, and a divider is a bigger thing
// than everything else in this file put together. What the design does
// instead is what the `baud` port is for: a design that needs two or three
// rates selects between constants, and one that needs any rate puts a
// divider there. Since the transmit and the receive halves share one
// divisor here, and they are wired to each other, the round trip works at
// whatever the constant is and a host asking for 9600 gets 115163 without
// noticing.
//
// Pins: testdata/fpga/cynthion/usb_cdc_uart.rcf.
// Sources: ip/usb_cdc_acm/rtl/*.v, ip/usb_device_ulpi/rtl/usb_ulpi_link.v,
//          ip/usb_device_fs/rtl/usb_ctrl_ep.v and ip/uart/rtl/*.v.

module usb_cdc_uart #(
    // How many clocks the core is held in reset after configuration. A
    // testbench has no use for more than a few; the board gets sixteen,
    // which is 0.27 us at 60 MHz.
    parameter integer POR = 16,
    // Cycles of an idle bus at J before the device's answer goes out, which
    // ULPI 1.1 Table 10 allows a full-speed Link between 7 and 18 of.
    parameter [6:0] TURNAROUND = 7'd9,
    // Clock cycles per serial bit: 60 MHz over 115200, rounded.
    parameter integer CLK_DIV = 521
) (
    input  wire clk,             // A8, the 60.000 MHz oscillator

    // The auxiliary transceiver, all on the die's right edge but D16.
    inout  wire [7:0] ulpi_data, // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire ulpi_dir,        // E16
    input  wire ulpi_nxt,        // F15
    output wire ulpi_stp,        // E15
    output wire ulpi_rst_n,      // J13, active low at the ball
    output wire ulpi_clk,        // D16, the clock the board says we owe it

    output wire led0_n,          // the transceiver is ready
    output wire led1_n,          // THE HOST CONFIGURED IT
    output wire led2_n,          // A BYTE WENT OUT TO THE UART
    output wire led3_n,          // heartbeat
    output wire led4_n,          // A BYTE CAME BACK FROM THE UART
    output wire led5_n           // the UART's transmit line
);
    // -----------------------------------------------------------------
    // The power-on reset: a one walked along a shift register, so the core
    // is held in reset for `POR` clocks — 0.27 us at 60 MHz, well inside
    // the 5 us the core then holds the transceiver's own reset pin for.
    //
    // `usb_ulpi_device.v`'s header says why this is a shift register and not
    // `rst_n` tied high: an ECP5 releases every flip-flop into its `REGSET`
    // state and tying it high would work on the part, but a simulator has no
    // `REGSET` and every register of the core would stay unknown for ever.
    // -----------------------------------------------------------------
    reg [POR-1:0] por = {POR{1'b0}};
    always @(posedge clk) begin
        por <= {por[POR-2:0], 1'b1};
    end
    wire reset_done = por[POR-1];

    // -----------------------------------------------------------------
    // The serial port.
    // -----------------------------------------------------------------
    wire [7:0] data_o;
    wire       data_oe;
    wire [6:0] address;
    wire       configured;
    wire       usb_reset;
    wire       phy_ready;

    // The port's bytes.
    wire [7:0] out_data;
    wire       out_valid;
    wire       out_last;
    wire       out_ready;
    wire [7:0] in_data;
    wire       in_valid;
    wire       in_ready;
    wire       in_commit;

    // What the host asked the line to be, and the control lines it
    // asserted. Nothing here acts on them — "THE BAUD DIVISOR" above says
    // why — and they are brought out to named wires rather than left
    // unconnected so that a person adding a divider or an LED for DTR has
    // them in front of them.
    wire [31:0] baud;
    wire [7:0]  char_format;
    wire [7:0]  parity;
    wire [7:0]  data_bits;
    wire        dtr;
    wire        rts;

    // THE TURNAROUND, which is the top level's whole job on this bus: the
    // link says when it owns the bus and this makes that eight pads. There
    // is no register in the way, so the pads let go in the same cycle the
    // link does.
    assign ulpi_data = data_oe ? data_o : 8'bz;

    // The interface clock the board asks the FPGA to provide. `clk_dir='o'`
    // and 60 MHz both ways, so there is nothing to make: no PLL.
    assign ulpi_clk = clk;

    // `VENDOR_ADDR` / `VENDOR_DATA` are this board's one register and not
    // ULPI's: a Cynthion crosses DP and DM between the transceiver and the
    // connector and register 39h bit 1 of the Microchip USB3343 undoes it.
    // `usb_ulpi_device.v`'s header has the three sources that agree on it.
    usb_cdc_acm_ulpi #(
        .TURNAROUND  (TURNAROUND),
        .VENDOR_ADDR (6'h39),
        .VENDOR_DATA (8'h06)
    ) u_acm (
        .clk60        (clk),
        .rst_n        (reset_done),
        .ulpi_data_i  (ulpi_data),
        .ulpi_data_o  (data_o),
        .ulpi_data_oe (data_oe),
        .ulpi_dir     (ulpi_dir),
        .ulpi_nxt     (ulpi_nxt),
        .ulpi_stp     (ulpi_stp),
        .ulpi_rst_n   (ulpi_rst_n),
        .address      (address),
        .configured   (configured),
        .usb_reset    (usb_reset),
        .phy_ready    (phy_ready),
        .out_data     (out_data),
        .out_valid    (out_valid),
        .out_last     (out_last),
        .out_ready    (out_ready),
        .in_data      (in_data),
        .in_valid     (in_valid),
        .in_ready     (in_ready),
        .in_commit    (in_commit),
        .baud         (baud),
        .char_format  (char_format),
        .parity       (parity),
        .data_bits    (data_bits),
        .dtr          (dtr),
        .rts          (rts)
    );

    // -----------------------------------------------------------------
    // THE BRIDGE, AND WHY IT CARRIES ONE BYTE AT A TIME
    // -----------------------------------------------------------------
    // `uart_rx` has **no back-pressure at all**: `rx_valid` is one cycle wide
    // and a byte nobody takes in that cycle is gone, which its header says in
    // so many words. The IN endpoint, on the other side, holds its packet
    // until the host asks for it, and how long that is belongs to the host's
    // scheduler and not to this design. So something has to stand between a
    // receiver that cannot wait and an endpoint that makes things wait.
    //
    // The usual answer is a FIFO, and `ip/fifo_sync` is a block in this
    // library for exactly this. **It cannot be placed on this part**, and
    // that is a gap in the FPGA backend rather than a property of the board:
    // `fifo_sync`'s storage is an array read by a variable index, which
    // becomes a distributed RAM, and `src/fpga/devices/ecp5.dev` declares
    // `bel TRELLIS_DPR16X4 lutram` with **no `count`** — so `fpga::place`
    // sees zero sites of it and refuses, whatever the depth:
    //
    //     error: the design needs 2 `lutram` site(s) and the part has 0
    //
    // Which leaves two honest choices: write a queue out of named registers
    // in this file, or arrange for a queue not to be needed. This design does
    // the second, because the second is **provably** lossless and the first
    // is only lossless up to a depth:
    //
    //   * a byte is handed to the transmitter only when nothing is in the
    //     loop and the return register is empty, so there is never more than
    //     one byte between `uart_tx`'s shifter and the IN endpoint;
    //   * so the whole return path is one byte and one flag, ten flip-flops,
    //     and no byte can be lost by anything;
    //   * and the USB OUT endpoint holds its packet and NAKs the host while
    //     it waits, which is what bulk flow control is for.
    //
    // What it costs is throughput: one character is a whole round trip — 87
    // microseconds of 8N1 plus however long the host takes to collect a
    // one-byte packet — where a bridge with a queue would have the UART
    // running back to back and the USB side overlapped with it. No figure is
    // quoted for either, because nothing here measured one. For a serial port
    // a person types at, and for a test that moves a few dozen bytes, the
    // round trip is the right trade; for a bridge to a **real** device it is
    // not, because
    // a real device's bytes arrive when they arrive and cannot be throttled
    // by anything at this end. Such a design needs the FIFO, and needs the
    // backend to be able to place one.
    wire [7:0] uart_rx_data;
    wire       uart_rx_valid;
    wire       uart_rx_error;
    wire       uart_tx_ready;
    wire       uart_txd;
    wire       uart_rxd;

    // The byte on its way back, and whether there is one. `in_flight` is a
    // byte given to the transmitter whose echo has not arrived yet.
    reg [7:0] echo;
    reg       echo_full;
    reg       in_flight;

    // WHICH SIGNAL THE THROTTLE GOES ON, WHICH IS NOT THE ONE IT LOOKS LIKE
    //
    // `uart_tx` takes a byte on **its own** handshake — `!busy_q && tx_valid`,
    // its header's words — so the condition that says "not yet" has to be on
    // `tx_valid` and not only on `out_ready`. This was
    //
    //     assign out_ready = uart_tx_ready & ~in_flight & ~echo_full;
    //     .tx_valid (out_valid)
    //
    // and **on the part it repeated bytes and lost bytes**: with `in_flight`
    // set the endpoint held its byte, `out_valid` stayed high, and the
    // transmitter — idle again and looking only at `tx_valid` — sent the same
    // byte again. Forty-eight bytes came back as fourteen distinct ones, some
    // of them seven times over, with two missing where a second character
    // overlapped the first. Simulation of the blocks could not have caught it:
    // it is this file's wiring and nothing else's.
    //
    // So `hand` is the whole of "there is a byte and the loop will take it",
    // it drives `tx_valid`, and `out_ready` is that **and** the transmitter
    // being idle, which is the one cycle the byte actually moves. Stating the
    // condition once and using it twice is also the only arrangement in which
    // the two cannot drift apart again.
    //
    // `out_ready` depending on `out_valid` is not a loop: every output of
    // `usb_bulk_ep`'s byte interface is a function of its own registers and no
    // input of it reaches one combinationally, which that module's header
    // states as a property of the block.
    wire hand = out_valid & ~in_flight & ~echo_full;
    assign out_ready = hand & uart_tx_ready;

    uart #(
        .CLK_DIV (CLK_DIV)
    ) u_uart (
        .clk      (clk),
        .rst_n    (reset_done),
        .tx_data  (out_data),
        .tx_valid (hand),
        .tx_ready (uart_tx_ready),
        .tx       (uart_txd),
        .rx       (uart_rxd),
        .rx_data  (uart_rx_data),
        .rx_valid (uart_rx_valid),
        .rx_error (uart_rx_error)
    );

    // THE LOOP. One net, and the header above says what it does and does not
    // establish.
    assign uart_rxd = uart_txd;

    // The byte goes to the IN endpoint as a packet of its own, because a
    // serial port must not make a keystroke wait for seven more. `in_commit`
    // with the byte is what sends a one-byte packet rather than waiting for
    // eight (`usb_bulk_ep` arms itself at eight either way).
    wire give = echo_full & in_ready;
    assign in_data   = echo;
    assign in_valid  = give;
    assign in_commit = give;

    always @(posedge clk or negedge reset_done) begin
        if (!reset_done) begin
            echo      <= 8'd0;
            echo_full <= 1'b0;
            in_flight <= 1'b0;
        end else begin
            // Handed to the transmitter.
            if (out_valid & out_ready) in_flight <= 1'b1;
            // Back out of the receiver. A framing error is not special-cased:
            // the byte such a frame produced is handed back anyway, and
            // `uart_rx_error` is deliberately left unread so that a broken
            // divisor shows as wrong bytes at the host rather than as
            // silence. Nothing on this board can cause one, since the two
            // halves share `CLK_DIV`.
            if (uart_rx_valid) begin
                echo      <= uart_rx_data;
                echo_full <= 1'b1;
                in_flight <= 1'b0;
            end
            // Taken by the endpoint.
            if (give) echo_full <= 1'b0;
        end
    end

    // -----------------------------------------------------------------
    // The heartbeat and the latches.
    // -----------------------------------------------------------------
    // The reduction spelling of `+ 1` that `clock_blink.v` explains.
    reg [25:0] count = 26'd0;
    wire [25:0] toggle;
    assign toggle[0] = 1'b1;

    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : carry
            assign toggle[i] = &count[i-1:0];
        end
    endgenerate

    always @(posedge clk) begin
        count <= count ^ toggle;
    end

    // Written `q <= q | event` and not `if (event) q <= 1'b1` because the
    // second infers a clock enable, and a slice's two flip-flops share one
    // `CE` wire.
    reg saw_configured = 1'b0;
    reg saw_tx         = 1'b0;
    reg saw_rx         = 1'b0;
    always @(posedge clk) begin
        saw_configured <= saw_configured | configured;
        saw_tx         <= saw_tx | (out_valid & out_ready);
        saw_rx         <= saw_rx | uart_rx_valid;
    end

    // Active low: a pin driven low lights one.
    assign led0_n = ~phy_ready;
    assign led1_n = ~saw_configured;
    assign led2_n = ~saw_tx;
    assign led3_n = ~count[25];
    assign led4_n = ~saw_rx;
    // The transmit line itself, so the LED is dark while the line idles
    // high and ball C11 carries the characters out of the part.
    assign led5_n = uart_txd;

endmodule
