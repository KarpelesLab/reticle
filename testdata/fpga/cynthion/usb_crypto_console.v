// SHA-256 and ChaCha20 on a Cynthion, behind its USB serial port.
//
// `ip/crypto/sha256` and `ip/crypto/chacha20` landed verified against
// FIPS 180-4, RFC 8439 and `purecrypto` — **entirely in simulation**. Both
// READMEs say so, and both name the experiment they were not doing. This is
// it: `crypto_console` on the auxiliary ULPI transceiver, so a
// `/dev/ttyACM*` the kernel's own `cdc_acm` driver binds answers
// `H abc` with a digest `sha256sum` agrees with.
//
// `testdata/fpga/cynthion/usb_cdc_uart.v` is the design this one is built
// after — the same transceiver, the same six LEDs, the same argument about
// why driving these pins is safe — and `examples/mos6502_monitor` is the
// other design on this board with a **command interface** over that
// console. This is the third thing of that shape and the first that
// computes something a published standard has an answer for.
//
// ===================================================================
// WHAT THERE IS TO LOOK AT
// ===================================================================
//
//     dmesg | tail
//     ls -l /dev/serial/by-id/
//     stty -F /dev/ttyACM1 115200 raw -echo
//     cat /dev/ttyACM1 &
//     echo 'H abc' > /dev/ttyACM1
//     printf '%s' abc | sha256sum
//
// Both terminators end a line, so `echo` and `printf 'H abc\r'` both work and
// a CRLF is one line; `crypto_console.v`'s header says why that is two rules
// and not one. `?` lists the commands and the port prints that list on its
// own the moment it is configured, so none of this needs a manual.
//
// `1209:0001` is pid.codes' test pair, the default of the core's `VID` and
// `PID`. **The number in `/dev/ttyACM*` is not fixed**: a Cynthion's own
// Apollo debugger is itself a CDC ACM device and is usually `ttyACM0`, so
// this one is normally `ttyACM1`. `/dev/serial/by-id/` names them, and
// `tests/usb_crypto_console.rs` finds the port by walking sysfs rather than
// by guessing a number.
//
// The baud rate is irrelevant and is set only because `stty` insists on
// one: there is no serial line inside this design, so nothing here divides
// anything. `usb_cdc_uart.v` next door is the one with a real 8N1 waveform
// on a pin, and `crypto_console_ulpi.v` says why this one ignores
// `dwDTERate`.
//
// The six LEDs are the diagnosis for when the port does not appear, and the
// first four are read the same way `usb_cdc_uart.v`'s are:
//
//     LED 0   THE TRANSCEIVER CAME UP         `phy_ready`, live
//     LED 1   THE HOST CONFIGURED IT          `configured`, latched
//     LED 2   A LINE WAS TYPED AT IT          latched
//     LED 3   heartbeat, 0.89 Hz              the clock runs
//     LED 4   AN ANSWER WENT BACK             latched
//     LED 5   A CORE HAS BEEN STARTED         live for the hash, latched for
//                                             the cipher — read the note
//
// Read from the bottom up, and `usb_ulpi_device.v`'s header has the whole
// ladder for LEDs 0, 1 and 3 and the bus below them. What is new here:
//
//   * **LED 1 lit, LED 2 dark** — the device enumerated and no line
//     terminator has been written to the port. That is the resting state:
//     opening a port is not typing at it.
//   * **LED 2 lit, LED 4 dark** — a line arrived and no answer came back,
//     which means the console is stuck waiting for a core. A core that
//     never finishes is the one failure this design can have that is not
//     visible as a wrong answer.
//   * **LED 4 lit** — the whole path worked at least once: a line in, a
//     core run, an answer out.
//   * **LED 5 is two signals ORed together and they behave differently**,
//     which is worth stating because the obvious reading of it is wrong.
//     `ip/crypto/sha256`'s `busy` is live: it is high while a message is
//     being hashed, so LED 5 flickers during `H`, `h`, `Z` and `X` and is
//     dark between them, and a long one makes it visibly dim rather than
//     off. `ip/crypto/chacha20`'s `active` is **not** live: it means "a
//     stream is open", a stream is opened by `start` and closed by nothing
//     but a reset or the counter running out, so it **latches** from the
//     first `e`, `E` or `X` of the session. So LED 5 blinks until a cipher
//     command is typed and is lit from then on.
//
//     That is a property of the block's port and not a defect in it —
//     `ip/crypto/chacha20/README.md` §3 says `active` is "a stream is open:
//     `start` has been seen and `exhausted` has not latched" — and it is
//     left as it is rather than papered over with a timer, because a
//     latched LED that says "the cipher has been used" is still a true
//     statement and inventing a rule for when a stream ends would be
//     inventing a rule this design has no reason to have.
//
// ===================================================================
// WHY DRIVING THESE PINS IS SAFE
// ===================================================================
//
// The same eight ULPI balls, the same clock and the same six LEDs
// `usb_ulpi_device.v` and `usb_cdc_uart.v` drive, for the same reasons and
// with the same argument; those files' headers have the long form and
// `usb_crypto_console.rcf` beside this one has every line's provenance. The
// short form: the data lines are released whenever `ulpi_dir` is high,
// which is the bus's own arbitration (ULPI 1.1 §3.3); every direction is
// the one Great Scott Gadgets' platform file gives it; the Type-C
// controllers, **the VBUS switches** and the pseudo-supply pins are left
// alone; and the `CONTROL` port the Apollo debugger lives on is a different
// transceiver on different balls and is not mentioned here, so loading this
// design cannot take the debugger away.
//
// ===================================================================
// TWO ANSWERS TO "HOW BIG IS THIS PART", AND THEY DIFFER BY A FACTOR OF TWO
// ===================================================================
//
// This design is **11 971 lookup tables** and 2 999 flip-flops: the serial
// port is about 1 230 of them, `ip/crypto/sha256` 3 205,
// `ip/crypto/chacha20` 5 978, and `crypto_console` itself 1 558.
//
// `src/fpga/devices/ecp5.dev` declares `count 12144` for an
// `ecp5-12f-CABGA256`, which is the LFE5U-12F's datasheet figure, so at the
// synthesis stage this design is at **98.6 per cent** of the part.
//
// **Placement reports 11 973 of 24 288 lut sites**, which is 49 per cent,
// and it is not wrong either: the sites come out of Project Trellis' own
// tile grid, and prjtrellis describes a **die**. An LFE5U-12F is an
// LFE5U-25F die with less of it guaranteed, so a fabric database that
// describes the silicon finds 24 288 lookup tables, 3 036
// `TRELLIS_DPR16X4` and 56 block RAMs on a part whose data sheet promises
// half of that.
//
// Nothing here depends on the difference — 11 971 fits inside 12 144 — and
// it is written down because a design that needed **more** than 12 144
// would synthesise, place, route and produce a working bitstream while
// using fabric Lattice does not guarantee on this part number, and nothing
// in the flow would say so. That is a note about the two databases rather
// than about this design, and the place to act on it is `src/`.
//
// The three decisions this design made expecting a full part are still the
// right ones and cost nothing: no FIFO anywhere (so no byte is ever held and
// a line may be any length), the key, nonce and counter **rotated** rather
// than parallel-loaded out of a staging register, and the digest read out of
// `ip/crypto/sha256`'s own holding register instead of copied into one here.
// `crypto_console.v`'s header has each of them with its cost.
//
// ===================================================================
// MOST OF THE AREA IS ADDERS WITH NO CARRY CELL, AND THAT IS THE CLOCK
// ===================================================================
//
// `src/fpga/trellis` describes `CCU2C` without a (ci, i0, i1, co) port map,
// so the flow reports `85 adder(s) stay generic` for this design and a
// 32-bit addition becomes a ripple of LUT4 about twenty-one levels deep.
// On an iCE40, where `SB_CARRY` is inferred, ChaCha20's quarter round is
// **5** levels of lookup table and SHA-256's T1 chain is **9**; here the
// mapper reports the design's longest path as **87**, which is ChaCha20's
// four chained 32-bit additions, and `ip/crypto/sha256`'s own is **39**.
//
// **There is no vendor timing model in this repository**, so "the clock
// closed" cannot be a slack number here: `reticle timing` says in its own
// help that its device numbers are placeholders. What a closed clock means
// for this design is therefore the stronger thing and not the weaker one —
// **the part computes the right answers at 60 MHz**, which a path missing
// 16.67 ns could not do, because a wrong bit anywhere in eighty-seven
// levels of logic changes the digest. `tests/usb_crypto_console.rs` is that
// measurement and the session quoted above is what it printed.
//
// Inferring the carry cell is the single change that would most move both
// the area and the achievable clock of this design. It is a change to
// `src/` and not to anything in `ip/`, and it is noted here and not made.
//
// Sources: ip/usb/usb_cdc_acm/rtl/*.v,
//          ip/usb/usb_device_ulpi/rtl/usb_ulpi_link.v,
//          ip/usb/usb_device_ulpi/rtl/usb_device_ulpi.v,
//          ip/usb/usb_device_fs/rtl/usb_ctrl_ep.v,
//          ip/crypto/sha256/rtl/*.v, ip/crypto/chacha20/rtl/*.v,
//          testdata/fpga/cynthion/crypto_console.v and
//          testdata/fpga/cynthion/crypto_console_ulpi.v.
// Pins:    testdata/fpga/cynthion/usb_crypto_console.rcf.
module usb_crypto_console #(
    // How many clocks the core is held in reset after configuration. A
    // testbench has no use for more than a few; the board gets sixteen,
    // which is 0.27 us at 60 MHz.
    parameter integer POR = 16,
    // Cycles of an idle bus at J before the device's answer goes out, which
    // ULPI 1.1 Table 10 allows a full-speed Link between 7 and 18 of.
    parameter [6:0] TURNAROUND = 7'd9
) (
    input  wire clk,             // A8, the 60.000 MHz oscillator

    // The auxiliary transceiver, all on the die's right edge but D16.
    inout  wire [7:0] ulpi_data, // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire ulpi_dir,        // E16
    input  wire ulpi_nxt,        // F15
    output wire ulpi_stp,        // E15
    output wire ulpi_rst_n,      // J13, active low at the ball
    output wire ulpi_clk,        // D16, the clock the board says we owe it

    output wire led0_n,          // THE TRANSCEIVER CAME UP
    output wire led1_n,          // THE HOST CONFIGURED IT
    output wire led2_n,          // A LINE WAS TYPED AT IT
    output wire led3_n,          // heartbeat
    output wire led4_n,          // AN ANSWER WENT BACK
    output wire led5_n           // A CORE IS WORKING
);
    // -----------------------------------------------------------------
    // The power-on reset: a one walked along a shift register, so the core
    // is held in reset for `POR` clocks — 0.27 us at 60 MHz, well inside
    // the 5 us the core then holds the transceiver's own reset pin for.
    //
    // `usb_ulpi_device.v`'s header says why this is a shift register and
    // not `rst_n` tied high: an ECP5 releases every flip-flop into its
    // `REGSET` state and tying it high would work on the part, but a
    // simulator has no `REGSET` and every register of the core would stay
    // unknown for ever.
    // -----------------------------------------------------------------
    reg [POR-1:0] por = {POR{1'b0}};
    always @(posedge clk) begin
        por <= {por[POR-2:0], 1'b1};
    end
    wire reset_done = por[POR-1];

    wire [7:0] data_o;
    wire       data_oe;
    wire [6:0] address;
    wire       configured;
    wire       usb_reset;
    wire       phy_ready;
    wire       hash_busy;
    wire       cipher_active;
    wire       saw_line;
    wire       saw_answer;

    // ===================================================================
    // WHAT THIS PORT REPORTS AS ITS LINE STATE
    // ===================================================================
    //
    // `serial_state` is `wSerialState` of the SERIAL_STATE notification
    // `ip/usb/usb_cdc_acm` sends: bit 0 is `bRxCarrier` (DCD), bit 1 is
    // `bTxCarrier` (DSR), and bits 2 to 6 are break, ring, framing, parity
    // and overrun (PSTN 1.2 §6.5.4 Table 31).
    //
    // **Both carriers, no errors**, which is what `usb_cdc_uart.v` reports
    // and for a stronger version of the same reason: there is no serial
    // line here at all, not even one eleven nets long, so there is nothing
    // that could be unplugged and no framing error that anything could
    // cause. A constant here is told to the host on **every open**, because
    // `ip/usb/usb_cdc_acm` sends a notification when the host opens the
    // port and not only when the state changes; that block's README §4
    // writes up what happened when it did not.
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
    crypto_console_ulpi #(
        .TURNAROUND  (TURNAROUND),
        .VENDOR_ADDR (6'h39),
        .VENDOR_DATA (8'h06)
    ) u_top (
        .clk60         (clk),
        .rst_n         (reset_done),
        .ulpi_data_i   (ulpi_data),
        .ulpi_data_o   (data_o),
        .ulpi_data_oe  (data_oe),
        .ulpi_dir      (ulpi_dir),
        .ulpi_nxt      (ulpi_nxt),
        .ulpi_stp      (ulpi_stp),
        .ulpi_rst_n    (ulpi_rst_n),
        .address       (address),
        .configured    (configured),
        .usb_reset     (usb_reset),
        .phy_ready     (phy_ready),
        .serial_state  (serial_state),
        .hash_busy     (hash_busy),
        .cipher_active (cipher_active),
        .saw_line      (saw_line),
        .saw_answer    (saw_answer)
    );

    // -----------------------------------------------------------------
    // The heartbeat.
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
    always @(posedge clk) begin
        saw_configured <= saw_configured | configured;
    end

    // Active low: a pin driven low lights one.
    assign led0_n = ~phy_ready;
    assign led1_n = ~saw_configured;
    assign led2_n = ~saw_line;
    assign led3_n = ~count[25];
    assign led4_n = ~saw_answer;
    assign led5_n = ~(hash_busy | cipher_active);

    // `address` and `usb_reset` are brought out of the module below and not
    // used here, which is deliberate: they are the two signals a person
    // adding a seventh diagnosis would want, and a wire with a name is
    // easier to find than a port with a blank beside it.
    wire [7:0] unused = {address, usb_reset};
endmodule
