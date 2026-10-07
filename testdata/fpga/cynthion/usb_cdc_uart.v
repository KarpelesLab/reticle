// A USB serial port on a Cynthion's AUX port, bridged to a UART:
// `ip/usb/usb_cdc_acm` wired to the auxiliary ULPI transceiver, its bytes
// handed to `ip/bus/uart`'s transmitter, and that transmitter's line
// **looped back into its own receiver** so that what a program writes to
// `/dev/ttyACM*` it reads back — having been serialised in between at the
// rate **and in the character format** the host set with `stty`.
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
//     LED 0   THE LINE CODING CAME FROM THE HOST   live
//     LED 1   THE HOST CONFIGURED IT               `configured`, latched
//     LED 2   A BYTE WENT OUT TO THE UART          latched
//     LED 3   heartbeat, 0.89 Hz                   the clock runs
//     LED 4   A BYTE CAME BACK FROM THE UART       latched
//     LED 5   THE UART'S TRANSMIT LINE             live, dark while idle
//
// Read from the bottom up, and `usb_ulpi_device.v`'s header has the whole
// ladder for LEDs 0, 1, 3 and the bus below them. What is new here:
//
//   * **LED 0 dark with LED 1 dark** — nothing has enumerated it and no
//     line coding has been set, which is also what a transceiver that never
//     came up looks like; `phy_ready` used to have this LED and
//     `usb_ulpi_device.v` still reports it on LED 0, so load that one to
//     tell the two apart.
//   * **LED 0 dark with LED 1 lit** — the device is enumerated and the wire
//     is **not** doing exactly what the host asked for. It is `baud_ok`
//     and `coding_ok` together, so either the rate was one
//     `uart_baud_div` refused and the divisor is still `CLK_DIV`, or the
//     framing was one `uart_line_coding` could not give exactly — sixteen
//     data bits, an undefined parity type, or one and a half stop bits,
//     which this UART sends as two. `stty -F /dev/ttyACM1 115200 cs8 -parenb
//     -cstopb` lights it. Note that **the wire is still being driven** in
//     every one of those cases, with the substitution each block's header
//     documents; the LED says "not exactly", not "not at all".
//   * **LED 1 lit, LED 2 dark** — the host enumerated the device and no
//     program has written to the port. That is the resting state: opening
//     the port is not writing to it.
//   * **LED 2 lit, LED 4 dark** — a byte reached `uart_tx` and no byte ever
//     came back out of `uart_rx`. Since the two are wired together inside
//     the part, that is a baud divisor wrong enough to break framing, or a
//     receiver that never saw the start bit — and nothing on the board can
//     cause it.
//   * **LED 4 lit** — the whole path worked at least once: USB out, a frame
//     down a wire, a frame back up, USB in.
//   * **LED 5** flickers while bytes flow and is dark otherwise, because the
//     line idles high and the LEDs are active low. It is also the one place
//     the serial data **leaves the part**: ball C11 carries the real
//     waveform — start bit, five to eight data bits, the parity bit if the
//     host asked for one, and one or two stop bits, at whatever rate the
//     host set — so an oscilloscope or a logic analyser on that LED sees the
//     characters, and sees the *framing* change when `stty` changes it.
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
// THE BAUD DIVISOR, WHICH IS NOW THE RATE THE HOST ASKED FOR
// ===================================================================
//
// `SET_LINE_CODING` carries `dwDTERate` and `ip/usb/usb_cdc_acm` brings it out
// on `baud`. This design **follows it**: `uart_baud_div` divides 60 000 000
// by that number and hands the quotient to `uart_frame`'s `div` port, so the
// waveform on ball C11 changes rate when a host changes rate.
//
// It did not, and the paragraph that used to be here said why not — that a
// run-time divider was a bigger thing than everything else in this file put
// together. It is not: `uart_baud_div` is a 33-bit subtract, four registers
// and a five-bit step counter, one quotient bit per clock for 32 clocks. The
// reason it was worth doing is the sentence that paragraph ended with: *"the
// round trip works at whatever the constant is and a host asking for 9600
// gets 115163 without noticing"*. That is a loopback agreeing with itself,
// and a loopback agreeing with itself is exactly what a wrong divisor looks
// like. Both ends of this design still move together, so the *echo* still
// cannot tell the rate — but **C11 can**, and a bit period on C11 is a
// number an instrument reads off the pin.
//
// `CLK_DIV` is still 521, and it is still 115200 baud at 60 MHz to 0.032 %.
// It is now the answer for a rate that cannot be divided to: `div` of zero
// means "use CLK_DIV" (`uart_tx.v`), and `uart_baud_div` drives DIV_RESET
// with `ok` low for a rate of zero, a rate too slow to express in sixteen
// bits, or one so fast that rounding would break framing. Its header has
// the error budget and the worked table; the short form is that a divisor
// under 30 is refused because half a clock in 30 is 1.67 % and 8N1's
// practical budget is about 2 %.
//
// ===================================================================
// THE FRAMING, WHICH IS ALSO NOW WHAT THE HOST ASKED FOR
// ===================================================================
//
// `SET_LINE_CODING` carries three more fields — `bCharFormat`,
// `bParityType` and `bDataBits` — and `ip/usb/usb_cdc_acm` brings all three
// out beside `baud`. This design **follows them too**:
// `uart_line_coding` turns the host's three bytes into `uart_frame`'s
// `cfg_data_bits`, `cfg_parity` and `cfg_stop`, and `uart_frame` is
// `uart_tx` and `uart_rx` with those on ports instead of parameters.
//
// They were stored, reported back verbatim by `GET_LINE_CODING`, and
// **ignored on the wire**: `stty -F /dev/ttyACM1 9600 parenb cs7` changed
// the rate and nothing else. That is the gap this closes, and it is the
// reason `ip/bus/uart` grew `uart_frame` at all — a format compiled into a
// parameter cannot follow a host.
//
// What a host may ask for and what it gets is `uart_line_coding`'s own
// header, in three tables. The short form: 5, 6, 7 and 8 data bits and all
// five parity types — none, odd, even, mark, space — are sent and checked
// exactly; one and two stop bits likewise; **one and a half stop bits are
// sent as two** and **sixteen data bits are sent as eight**, each with `ok`
// low so that this design can say so on LED 0. Nothing hangs and nothing
// silently does something else without saying it did.
//
// **The echo cannot tell the framing either**, for the same reason it
// cannot tell the rate: both halves of the loop take the same `cfg_*`
// wires, so a format that was wrong in both would still read back byte for
// byte. C11 is again the observable — a frame on that ball is one start
// bit, then as many data bits as `cs5`..`cs8` named, then a parity bit or
// not, then one or two stop bits, and the *number of bit periods between
// two start edges* is what changes when `stty` does. `tests/ip_library.rs`
// decodes exactly that by hand in Rust rather than with `uart_rx`, for the
// one defect a loopback structurally cannot see: it caught a transmitter
// that sent one stop bit where two were asked for, which the loopback
// passed happily, because a receiver samples the first stop bit and
// nothing after it.
//
// **LED 0 says whether the whole line coding came from the host.** It was
// the transceiver being ready, which LED 1 already implies once the host
// has configured the device; it then became the baud rate alone; it is now
// the rate **and** the framing, because "is this design doing exactly what
// I just set?" is the question this file exists to answer and the framing
// is three quarters of the setting.
//
// ===================================================================
// WHAT THE PART SAID, WHICH IS THE ONE FIELD A LOOPBACK CAN SHOW
// ===================================================================
//
// This was loaded, and so was the version of this file from before the
// framing was wired up, in the same session. The observable is the device
// file and nothing else: a character of *n* data bits carries *n* bits, so
// a byte with anything set above bit *n*-1 **cannot come back whole** —
// the transmitter drops those bits and the receiver has nothing to put in
// them. The host side is `termios` directly rather than `stty`, with
// `iflag` zero, so the kernel's own ISTRIP is not what narrows anything.
//
// Before, five line codings and one waveform:
//
//     cs8 n 1   sent 41 c1 ff 80   back 41 c1 ff 80
//     cs7 n 1   sent 41 c1 ff 80   back 41 c1 ff 80
//     cs6 n 1   sent 41 c1 ff 80   back 41 c1 ff 80
//     cs5 n 1   sent 41 c1 ff 80   back 41 c1 ff 80
//     cs8 e 2   sent 41 c1 ff 80   back 41 c1 ff 80
//
// After, each byte exactly `sent & ((1 << n) - 1)`:
//
//     cs8 n 1   sent 41 c1 ff 80   back 41 c1 ff 80
//     cs7 n 1   sent 41 c1 ff 80   back 41 41 7f 00
//     cs6 n 1   sent 41 c1 ff 80   back 01 01 3f 00
//     cs5 n 1   sent 41 c1 ff 80   back 01 01 1f 00
//
// And with the rate moved at the same time, since the two decodes are
// separate blocks: `9600 cs7 e 1` returns `41 41 7f 00`, `19200 cs5 o 2`
// returns `01 01 1f 00`, `230400 cs7 e 2` returns `41 41 7f 00`, and
// `1200 cs6 n 1` returns `01 01 3f 00`.
//
// **What that does and does not establish.** It establishes that
// `bDataBits` reaches the wire, and — because a parity bit one end inserts
// and the other does not expect moves the stop bit by a whole bit period —
// that both halves agree about whether there is a parity bit and where it
// sits, at every width, with one stop bit and with two. It does **not**
// establish the parity bit's *value* or the number of stop bits: those are
// symmetric in a loop and no host-side observation can reach them.
// `tests/ip_library.rs` is where they are checked, by a decoder written in
// Rust rather than by `uart_rx`, and `ip/bus/uart/README.md` §7 has the
// mutation that proves a receiver cannot do it.
//
// Pins: testdata/fpga/cynthion/usb_cdc_uart.rcf.
// Sources: ip/usb/usb_cdc_acm/rtl/*.v,
//          ip/usb/usb_device_ulpi/rtl/usb_ulpi_link.v,
//          ip/usb/usb_device_fs/rtl/usb_ctrl_ep.v and ip/bus/uart/rtl/*.v.

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

    output wire led0_n,          // the whole line coding came from the host
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
    // asserted. All four fields of the line coding are **acted on** —
    // `baud` through `uart_baud_div` and the other three through
    // `uart_line_coding` — and the two headers above say how. `dtr` and
    // `rts` are still only brought out to named wires, because this design
    // has no modem to assert them at and no spare LED to show them on.
    wire [31:0] baud;
    wire [7:0]  char_format;
    wire [7:0]  parity;
    wire [7:0]  data_bits;
    wire        dtr;
    wire        rts;

    // ===================================================================
    // WHAT THIS PORT REPORTS AS ITS LINE STATE
    // ===================================================================
    //
    // `serial_state` is `wSerialState` of the SERIAL_STATE notification
    // `ip/usb/usb_cdc_acm` sends: bit 0 is `bRxCarrier` (DCD), bit 1 is
    // `bTxCarrier` (DSR), and bits 2 to 6 are break, ring, framing, parity and
    // overrun (PSTN 1.2 §6.5.4 Table 31).
    //
    // **Both carriers, no errors.** The far end of this port's serial line is
    // `uart_rx`, eleven nets away inside the same die, so the carrier is
    // present and the data set is ready from the moment the part is
    // configured; there is no cable that could be unplugged and no modem that
    // could hang up. A design bridging to a **real** device would drive bit 0
    // from whatever tells it the far end is there.
    //
    // The error bits are clear and `uart_frame_rx`'s four error signals are
    // deliberately **not** wired to `bFraming`, `bParity`, `bBreak` and
    // `bOverRun`, for two reasons that both still hold. The first is that
    // **none of the four can fire on this board**: the two halves of the
    // loop take the same `div` and the same `cfg_*` wires, so there is no
    // framing or parity error to have; nothing holds the line low, so there
    // is no break; and `rx_ready` below is high in every cycle a character
    // can arrive, so there is no overrun. The second is the shape of the
    // signals: PSTN's error bits are levels a device sets and clears, and
    // these are one-cycle pulses. Turning a pulse into a level needs a rule
    // about when it goes away, and a design where the pulse cannot happen
    // has no way to choose that rule well.
    //
    // What a host does with it: `cdc_acm` keeps the last bitmap it was sent
    // in `ctrlin` and answers `TIOCMGET` out of it, so a program asking
    // this terminal for its modem lines is told there is a carrier and a
    // data set — **on every open**, because `ip/usb/usb_cdc_acm` sends a
    // notification when the host opens the port and not only when the state
    // changes. A constant here would otherwise be told to the host exactly
    // once and never again, which is the defect
    // `ip/usb/usb_cdc_acm/README.md` §4 writes up: whoever polled first got
    // the one packet and `TIOCMGET` then read `0x026`, with no DCD and no
    // DSR, on three consecutive opens.
    //
    // **It does not change whether the port opens**: `cdc_acm` has no
    // `carrier_raised` operation, so the terminal layer never waits for a
    // carrier on one of these whatever `clocal` says, and
    // `ip/usb/usb_cdc_acm/README.md` §5 says where an earlier round got that
    // wrong. `tests/usb_cdc_acm.rs` reads the ten bytes off endpoint 82h.
    wire [6:0] serial_state = 7'b000_0011;

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
        // Left open. It used to be LED 0; `configured` on LED 1 cannot be
        // true without it, and LED 0 now answers a question nothing else
        // in this design does.
        .phy_ready    (),
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
        .rts          (rts),
        .serial_state (serial_state)
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
    // The usual answer is a FIFO, and `ip/memory/fifo_sync` is a block in this
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
    // running back to back and the USB side overlapped with it.
    //
    // **A wider USB packet does nothing for this**, and that is worth
    // saying where somebody will look for it: the endpoints hold 64 bytes
    // now, and this bridge still hands one byte to `uart_tx` per round
    // trip, so the packets it sends are one byte long whatever
    // `wMaxPacketSize` says. The throughput figures in
    // `ip/usb/usb_cdc_acm/README.md` §5 are measured on the bulk loopback
    // of `usb_ulpi_device.v`, which has no UART in the way, for exactly
    // that reason. For a serial port a person types at, and for a test that
    // moves a few dozen bytes, the round trip is the right trade; for a
    // bridge to a **real** device it is not, because a real device's bytes
    // arrive when they arrive and cannot be throttled by anything at this
    // end. Such a design needs the FIFO, and needs the backend to be able
    // to place one.
    wire [7:0] uart_rx_data;
    wire       uart_rx_valid;
    wire       uart_rx_frame_error;
    wire       uart_rx_parity_error;
    wire       uart_rx_break;
    wire       uart_rx_overrun;
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

    // 60 000 000 / `baud`, rounded to nearest, or CLK_DIV when that is not
    // a number a UART can keep time with.
    wire [15:0] baud_div;
    wire        baud_ok;

    uart_baud_div #(
        .CLK_HZ    (32'd60_000_000),
        .DIV_RESET (CLK_DIV[15:0])
    ) u_baud (
        .clk   (clk),
        .rst_n (reset_done),
        .rate  (baud),
        .div   (baud_div),
        .ok    (baud_ok),
        .busy  ()
    );

    // The other three fields of the line coding, turned into the three
    // numbers the UART's format ports take. Combinational: the host's bytes
    // are already registers inside `usb_cdc_acm`, so there is nothing here
    // to clock.
    wire [3:0] cfg_data_bits;
    wire [2:0] cfg_parity;
    wire [1:0] cfg_stop;
    wire       coding_ok;

    uart_line_coding u_coding (
        .char_format   (char_format),
        .parity        (parity),
        .data_bits     (data_bits),
        .cfg_data_bits (cfg_data_bits),
        .cfg_parity    (cfg_parity),
        .cfg_stop      (cfg_stop),
        .ok            (coding_ok)
    );

    // `uart_frame` and not `uart`: the format is on ports here because a
    // host chooses it, and `uart` is the same pair with it compiled in.
    // `ip/bus/uart/README.md` §2 is why there are two of them.
    uart_frame #(
        .CLK_DIV (CLK_DIV)
    ) u_uart (
        .clk             (clk),
        .rst_n           (reset_done),
        .div             (baud_div),
        .cfg_data_bits   (cfg_data_bits),
        .cfg_parity      (cfg_parity),
        .cfg_stop        (cfg_stop),
        .tx_data         (out_data),
        .tx_valid        (hand),
        .tx_ready        (uart_tx_ready),
        .tx              (uart_txd),
        .rx              (uart_rxd),
        // The return register being empty is the whole of this design's
        // receive back-pressure, and the throttle above guarantees it is
        // empty whenever a character can arrive — so this is high every
        // time it matters and `uart_rx_overrun` can never fire. Driving it
        // from the real condition rather than tying it to one is still the
        // right wiring: if somebody changes the throttle, the receiver will
        // say that a byte was dropped instead of dropping it silently.
        .rx_ready        (~echo_full),
        .rx_data         (uart_rx_data),
        .rx_valid        (uart_rx_valid),
        .rx_frame_error  (uart_rx_frame_error),
        .rx_parity_error (uart_rx_parity_error),
        .rx_break        (uart_rx_break),
        .rx_overrun      (uart_rx_overrun)
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

    // And the other handshake, which `uart_frame_rx` now has: `rx_valid`
    // stays high until a cycle in which `rx_ready` is high too, so what
    // takes the character is that cycle and not `rx_valid` on its own.
    // Written as the conjunction rather than as `uart_rx_valid` alone
    // because the two differ the moment anybody changes the throttle: with
    // `echo_full` set, `rx_valid` would still be high and the byte would be
    // latched into `echo` over and over, which is the repeated-byte defect
    // this file already carries one account of.
    wire take = uart_rx_valid & ~echo_full;

    always @(posedge clk or negedge reset_done) begin
        if (!reset_done) begin
            echo      <= 8'd0;
            echo_full <= 1'b0;
            in_flight <= 1'b0;
        end else begin
            // Handed to the transmitter.
            if (out_valid & out_ready) in_flight <= 1'b1;

            // Taken by the endpoint. **This comes before the receive below on
            // purpose**, so that the two happening at once leaves
            // `echo_full` set and the new byte in `echo`, which is right:
            // `in_data` is the register, so the endpoint took the old byte,
            // and the new one has not been handed to anybody yet. Written the
            // other way round, the last assignment would win and the new byte
            // would be marked empty — a byte lost.
            //
            // They cannot happen at once as this design stands, and that is
            // exactly why the order is worth stating: `give` needs
            // `echo_full` and `take` needs it clear, so the two are now
            // mutually exclusive by construction as well as by the
            // invariant that `hand` will not start a character unless
            // `echo_full` and `in_flight` are both clear. The invariant is
            // real and it is also two lines away from whoever changes the
            // throttle.
            if (give) echo_full <= 1'b0;

            // Back out of the receiver. A framing or parity error is not
            // special-cased: the byte such a frame produced is handed back
            // anyway, and the four error signals are deliberately left
            // unread so that a broken divisor or a broken format shows as
            // wrong bytes at the host rather than as silence. Nothing on
            // this board can raise one, since the two halves share `div`
            // and share every `cfg_*` wire; "WHAT THIS PORT REPORTS AS ITS
            // LINE STATE" above says the same about `wSerialState`.
            if (take) begin
                echo      <= uart_rx_data;
                echo_full <= 1'b1;
                in_flight <= 1'b0;
            end
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
        saw_rx         <= saw_rx | take;
    end

    // Active low: a pin driven low lights one.
    // Live rather than latched: it is the answer to "is this design using
    // the line coding I just set?", and a latch would answer "did it
    // ever". Both halves of the answer have to hold at once, so it is the
    // conjunction of the divider's `ok` and the format decode's.
    assign led0_n = ~(baud_ok & coding_ok);
    assign led1_n = ~saw_configured;
    assign led2_n = ~saw_tx;
    assign led3_n = ~count[25];
    assign led4_n = ~saw_rx;
    // The transmit line itself, so the LED is dark while the line idles
    // high and ball C11 carries the characters out of the part.
    assign led5_n = uart_txd;

endmodule
