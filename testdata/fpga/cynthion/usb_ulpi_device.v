// A USB full-speed device on a Cynthion's AUX port: `ip/usb_device_ulpi`
// wired to the auxiliary ULPI transceiver, with the bus turnaround in the
// top level where the three-state buffers belong, and **endpoint 1 looped
// back on itself** so that the device has something to do once a host has
// finished enumerating it.
//
// ===================================================================
// WHAT THERE IS TO LOOK AT, AND WHY IT IS NOT AN LED
// ===================================================================
//
// **The observable is `lsusb` on the machine the AUX port is plugged into.**
// Either `1209:0001` appears with an eighteen-byte device descriptor or it
// does not, and no eye is involved:
//
//     lsusb -d 1209:0001 -v
//
// `1209:0001` is pid.codes' test pair, the default of the core's `VID` and
// `PID` parameters. That is the whole point of this design and the reason it
// is the first thing in this project a test could assert on its own.
//
// The **second** observable is bytes, and it needs no kernel driver either.
// The configuration descriptor declares one vendor-specific interface with a
// bulk OUT and a bulk IN on endpoint 1, so no class driver claims it and a
// program may claim the interface for itself and move bytes:
//
//     cargo test --features program -- --ignored usb_endpoint_one_loops
//
// which is `tests/usb_loopback.rs`. What the design does with those bytes is
// send them straight back: `out_*` wired into `in_*`, one packet at a time,
// which is the whole of it below. Anything the host writes to endpoint 1 OUT
// it reads back from endpoint 1 IN, in order, with packet boundaries where it
// put them. A loopback cannot carry a **zero-length** packet — an OUT of no
// bytes hands nothing to the interface, so there is nothing to hand back —
// and `ip/usb_device_fs`'s tests cover that direction instead.
//
// The **third** observable is in the same bytes and costs one flip-flop: the
// byte on its way back is XORed with `zero_probe`, a register whose data
// input is the constant zero. See "THE CONSTANT-ZERO PROBE" below. If the
// constant a bitstream writes for that pin is right the byte is unchanged and
// the loopback is byte-identical; if that flip-flop ever comes up holding a
// one, every returned byte is its own complement and the test above fails
// naming this.
//
// The six LEDs are the **diagnosis** for when it does not appear, and they
// are latched rather than level, so what a person sees is what happened:
//
//     LED 0   the transceiver is ready       `phy_ready`
//     LED 1   THE HOST CONFIGURED IT         `configured`, latched
//     LED 2   A BYTE WENT THROUGH ENDPOINT 1 latched
//     LED 3   heartbeat, 0.89 Hz             the clock runs
//     LED 4   a USB bus reset was seen       latched
//     LED 5   the transceiver drove the bus   latched
//
// LED 2 used to be "the host assigned an address", which LED 1 already
// implies: a device is not configured until it has been addressed. It is the
// data endpoint now, which nothing else can report.
//
// Read from the bottom up, each LED is the precondition for the one above:
//
//   * **LED 3 dark or frozen** — no clock, and nothing else means anything.
//   * **LED 5 dark** — the transceiver has never taken the bus, so it has
//     never said anything. Most likely it has no clock: `clk_dir='o'` means
//     the FPGA owes it the 60 MHz on D16, and an unclocked ULPI transceiver
//     holds `dir` asserted on purpose (ULPI 1.1 §3.12) — which would light
//     LED 5 rather than leave it dark, so LED 5 dark is stranger than that
//     and points at the reset (J13) or at the pins.
//   * **LED 0 dark** — the start-up sequence never finished. The core writes
//     Function Control, waits for the transceiver's own reset, writes it
//     again and **reads it back**, and only then reports `phy_ready`; so
//     LED 0 dark with LED 5 lit means the register conversation is wrong,
//     which is the half of ULPI a transceiver model written from the same
//     specification as the core cannot falsify.
//   * **LED 4 dark with LED 0 lit** — the transceiver is ready and the host
//     is not talking. Either nothing is plugged into the AUX port, or the
//     host never saw a device attach, which is the `TermSelect` bit of
//     Function Control: the 1.5 kOhm pull-up on D+ is a register bit here
//     and not a pin.
//   * **LED 1 dark with LED 4 lit** — the host reset the bus and then gave
//     up, which is `GET_DESCRIPTOR` going unanswered or answered wrong, or
//     `SET_ADDRESS` or `SET_CONFIGURATION` not taking.
//   * **LED 1 lit** — the host got through `SET_ADDRESS` and
//     `SET_CONFIGURATION`. At that point it is in `lsusb`.
//   * **LED 2 lit** — a byte the host wrote to endpoint 1 OUT was handed to
//     this file and given back to endpoint 1 IN. It says nothing about
//     whether the host read it back, which only the host can say; it says
//     the data endpoint's OUT side works and its byte interface moved.
//
// ===================================================================
// WHY DRIVING THESE PINS IS SAFE
// ===================================================================
//
// `bidir_bus.v` has the long form; this design is the same eight balls used
// for what they are for. The short form:
//
// 1. the eight data lines are released whenever `ulpi_dir` is high, which is
//    the bus's own arbitration (ULPI 1.1 §3.3), and `usb_ulpi_link` drives
//    `ulpi_data_oe` low in that case — the tri-state expression below is
//    what turns that into eight pads;
// 2. every pin's direction is the one Great Scott Gadgets' platform file
//    gives it, and `default_usb_connection = "aux_phy"` means this is the
//    port their own gateware puts a USB device on;
// 3. the Type-C controllers, the VBUS switches and the pseudo-supply pins
//    are left alone, which is what every gateware in the Cynthion
//    repository does with them;
// 4. the `CONTROL` port, where the Apollo debugger this board is programmed
//    over lives, is a different transceiver on different balls and is not
//    mentioned here. Loading this design cannot take the debugger away.
//
// ===================================================================
// WHAT IS NOT HERE
// ===================================================================
//
//   * **a reset that is only a power-on reset.** There is no reset button:
//     `POR` clocks a one along a shift register and the core is held in
//     reset until it arrives. Tying `rst_n` high instead would also have
//     worked on the part, because an ECP5 releases every flip-flop into its
//     `REGSET` state at the end of configuration and
//     `src/fpga/devices/ecp5.dev` gives them all `REGSET="RESET"` — but it
//     would be a design that cannot be simulated, since a simulator has no
//     `REGSET` and every register of the core would stay unknown forever.
//     A reset pulse makes the same design checkable, which is worth
//     sixteen flip-flops.
//
//     The core's reset is **asynchronous** (`negedge rst_n`), so this is
//     also the first design on this family to use one: 40 slices carry a
//     routed `LSR` and their `SRMODE`, which "What remains" in
//     `docs/fpga-trellis.md` said could not be written. It could not be
//     before two flip-flops sharing a slice were made to agree about their
//     shared control wires, because that is what decides which of a tile's
//     two reset muxes carries the signal.
//   * **`SLEWRATE=FAST`**, which every ULPI pin of this board's own
//     bitstreams asks for and this backend writes for none. It is an edge
//     rate on a 60 MHz bus, so it is the first thing to suspect if the
//     device is intermittent rather than absent.
//   * **`nxt` reaches the core** and `stp` comes from it; neither is a top
//     level decision.
//
// Pins: testdata/fpga/cynthion/usb_ulpi_device.rcf.
// Sources: ip/usb_device_ulpi/rtl/*.v and ip/usb_device_fs/rtl/usb_ctrl_ep.v.

module usb_ulpi_device #(
    // How many clocks the core is held in reset after configuration. A
    // testbench has no use for more than a few; the board gets sixteen,
    // which is 0.27 us at 60 MHz.
    parameter integer POR = 16,
    // Cycles of an idle bus at J before the device's answer goes out, which
    // ULPI 1.1 Table 10 allows a full-speed Link to spend between 7 and 18
    // of. It is a parameter here because it is the one number in this design
    // that a board can argue with, and sweeping it is one bitstream per
    // value.
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

    output wire led0_n,          // the transceiver is ready
    output wire led1_n,          // THE HOST CONFIGURED IT
    output wire led2_n,          // A BYTE WENT THROUGH ENDPOINT 1
    output wire led3_n,          // heartbeat
    output wire led4_n,          // a USB bus reset was seen
    output wire led5_n           // the transceiver drove the bus
);

    wire [7:0] data_o;
    wire       data_oe;
    // The address the host assigned. Nothing at this level needs it — LED 1
    // covers it, since a device is not configured until it is addressed —
    // and it stays here because a design built on this one will want it.
    wire [6:0] address;
    wire       configured;
    wire       usb_reset;
    wire       phy_ready;

    // Endpoint 1's byte interface.
    wire [7:0] out_data;
    wire       out_valid;
    wire       out_last;
    wire       in_ready;

    // THE TURNAROUND, which is the top level's whole job on this bus: the
    // link says when it owns the bus and this makes that eight pads. There
    // is no register in the way, so the pads let go in the same cycle the
    // link does.
    assign ulpi_data = data_oe ? data_o : 8'bz;

    // The interface clock the board asks the FPGA to provide. `clk_dir='o'`
    // and 60 MHz both ways, so there is nothing to make: no PLL.
    assign ulpi_clk = clk;

    // ===================================================================
    // THE CONSTANT-ZERO PROBE, AND WHY IT IS ONE FLIP-FLOP IN THE DATA PATH
    // ===================================================================
    //
    // **A flip-flop whose data input is the constant zero must hold zero.**
    // On this family that is not free and it is not obvious: a fabric
    // flip-flop takes its data from the slice's `M` wire, an unrouted slice
    // input on an ECP5 reads as a **one**, and until
    // `techcells::drive_constant_data` nothing was routed there — so such a
    // register came up **set**. `reg [2:0] stage` for four states read 5,
    // every `case` label missed, and a USB device did not enumerate for eight
    // rounds of looking somewhere else. The fix builds the constant out of a
    // lookup table with an `INIT` of all zeros and every input tied high,
    // which is what Lattice's own packer writes; `docs/fpga-trellis.md` has
    // it read out of their bitstreams at absolute frame positions.
    //
    // What was missing was a **part**. A constant *one* is confirmed in
    // silicon by this very design — `usb_ulpi_link`'s `rst_q` releases the
    // transceiver's reset pin, and a zero there means no host sees anything
    // — but no design that has run held a constant *zero*, because the
    // registers that used to supply one **were** the defect and narrowing
    // them removed them.
    //
    // So one goes here, in the one place on this board where a register bit
    // is readable by a program rather than by an eye: the byte endpoint 1
    // hands back. `zero_probe` is a flip-flop whose data input is `1'b0` and
    // nothing else, and the returned byte is XORed with it. Right, the XOR
    // is with zero and the loopback is **byte-identical**, so
    // `tests/usb_loopback.rs` passes exactly as it did before. Wrong, and
    // **every byte comes back as its own complement** — 256 bytes of it, in
    // 32 packets — which that test reports by name.
    //
    // **Why the initialiser is `1'b1` and not `1'b0`.** A compiler may
    // delete a flip-flop whose value is a known constant, and a test that
    // cannot fail is worse than no test. `synth::opt::FfOpt` does exactly
    // that — and only when the register's value *before* its first clock
    // agrees with its data. Initialised to one and clocked to zero, this
    // register is not constant in any language a compiler may reason in: it
    // holds one until the first edge and zero for ever after, so keeping it
    // is not a favour, it is the semantics. That is the whole of what makes
    // the probe survive, and `tests/fpga_trellis.rs`'s
    // `the_usb_devices_constant_zero_probe_survives_synthesis`
    // asserts the flip-flop and its constant driver are in the netlist so
    // that a future optimisation cannot quietly turn this test green for
    // ever.
    //
    // On the part the initialiser is a fiction and does not matter: an ECP5
    // releases every flip-flop into its `REGSET` state, which
    // `src/fpga/devices/ecp5.dev` makes `RESET`, so `zero_probe` starts at
    // zero and the first clock has to *keep* it there. The power-on reset
    // below gives it sixteen clocks before the core leaves reset, and nothing
    // reads the byte for milliseconds after that.
    //
    // It costs one flip-flop and **one** extra lookup table, not nine: the
    // eight XORs fold into inputs the endpoint's own lookup-table cover was
    // not using, and the `const0` driver is shared. **It is not a dead
    // register**: `zero_probe` reaches sixty-four lookup tables of the IN
    // data path and its value leaves the part in every byte the host reads.
    reg zero_probe = 1'b1;
    always @(posedge clk) begin
        zero_probe <= 1'b0;
    end

    // The byte on its way back to the host, and the only thing between the
    // OUT buffer and the IN buffer. Still combinational — eight XORs, which
    // the lookup-table cover absorbs — so this is logic between two
    // flip-flops and not a register, and the flow control below is unchanged.
    wire [7:0] loop_data = out_data ^ {8{zero_probe}};

    // ===================================================================
    // THE ONE REGISTER THAT IS THIS BOARD'S AND NOT ULPI'S
    // ===================================================================
    //
    // **This board exchanges DP and DM between the transceiver and the
    // connector, and the transceiver has a bit that undoes it.** Without
    // that bit the 1.5 kOhm pull-up `TermSelect` connects to the
    // transceiver's DP reaches the receptacle's D-, a host detects a
    // low-speed device where a full-speed one was asked for, and every
    // symbol after that is inverted. `docs/fpga-trellis.md` has the
    // measurements that said so before the cause was known: the
    // transceiver reporting its own D+ high in the same instant the host
    // reported low speed.
    //
    // Three sources, and they agree:
    //
    //   * the part is a **Microchip USB3343**, read off the board's Vendor
    //     ID register (`0424h`) and confirmed by the reference designator
    //     `U11 USB3343-CP` in Great Scott Gadgets' own PCB;
    //   * *USB334x Data Sheet* DS00002646A, Table 2-2 and Figure 2-2:
    //     **pin 13 is DP and pin 14 is DM**. The PCB wires pin 13 to the
    //     receptacle's D- (`A7`/`B7`) and pin 14 to its D+ (`A6`/`B6`),
    //     through the common-mode choke `FL1`, so the pair is crossed on
    //     purpose — both of the receptacle's pairs are tied together, so
    //     no plug orientation and no Type-C controller comes into it;
    //   * the same datasheet §7.1.3.5, register **39h** "USB IO & Power
    //     Management", bit 1 `SwapDP/DM`: *"When asserted, the DP and DM
    //     pins of the USB transceiver are swapped. This bit can be used to
    //     prevent crossing the DP/DM traces on the board."* Its reset value
    //     is `04h`, so `06h` is that default with bit 1 set.
    //
    // And Great Scott Gadgets' own gateware writes exactly that byte to
    // exactly that address, in `cynthion/python/src/gateware/platform/
    // core.py`:
    //
    //     ulpi_extra_registers = {
    //         0x39: 0b000110 # USB3343: swap D+ and D- to match the
    //                        # hardware design
    //     }
    //
    // The core writes it after the transceiver's own reset and before
    // `TermSelect`, and reads it back; see `usb_ulpi_link`'s parameter.
    usb_device_ulpi #(
        .TURNAROUND  (TURNAROUND),
        .VENDOR_ADDR (6'h39),
        .VENDOR_DATA (8'h06)
    ) u_dev (
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
        // ===============================================================
        // THE LOOPBACK, which is the whole of what this design is for once
        // it is enumerated.
        // ===============================================================
        //
        // `out_ready` is `in_ready` and nothing else: a byte leaves the OUT
        // buffer exactly when the IN buffer has room for it, so the endpoint
        // NAKs the host rather than dropping anything, and the flow control
        // is one wire. `in_commit` on the last byte of the packet is what
        // sends a short packet as a short packet instead of waiting for
        // eight bytes that are not coming.
        //
        // **There is no register in this path**, and there must not be: both
        // sides of the interface are functions of the endpoint's registers
        // alone, so wiring one straight into the other is combinational
        // logic between two flip-flops and not a loop. `ip/usb_device_fs`'s
        // `usb_bulk_ep` header states that as a property of the block, and
        // `tests/ip_library.rs` drives this same wiring in simulation.
        //
        // `in_data` is `loop_data`, which is `out_data` XORed with the
        // constant-zero probe above — the same byte when the constant a
        // bitstream writes for a flip-flop's data pin is right, and its
        // complement when it is not.
        .out_data     (out_data),
        .out_valid    (out_valid),
        .out_last     (out_last),
        .out_ready    (in_ready),
        .in_data      (loop_data),
        .in_valid     (out_valid),
        .in_ready     (in_ready),
        .in_commit    (out_valid & in_ready & out_last)
    );

    // The power-on reset: a one walked along a shift register, so the core
    // is held in reset for `POR` clocks — 0.27 us at 60 MHz, which is well
    // inside the 5 us the core then holds the transceiver's own reset pin
    // for. Sixteen flip-flops with no enable and no reset of their own.
    reg [POR-1:0] por = {POR{1'b0}};
    always @(posedge clk) begin
        por <= {por[POR-2:0], 1'b1};
    end
    wire reset_done = por[POR-1];

    // The heartbeat, so that a dark board can be told from a stopped one.
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

    // The latches. Written `q <= q | event` and not `if (event) q <= 1'b1`
    // because the second infers a clock enable, and a slice's two flip-flops
    // share one `CE` wire; `fpga::place` now knows, but a latch has no use
    // for one either way.
    reg saw_configured = 1'b0;
    reg saw_bytes      = 1'b0;
    reg saw_reset      = 1'b0;
    reg saw_dir        = 1'b0;
    always @(posedge clk) begin
        saw_configured <= saw_configured | configured;
        saw_bytes      <= saw_bytes | (out_valid & in_ready);
        saw_reset      <= saw_reset | usb_reset;
        saw_dir        <= saw_dir | ulpi_dir;
    end

    // Active low: a pin driven low lights one.
    assign led0_n = ~phy_ready;
    assign led1_n = ~saw_configured;
    assign led2_n = ~saw_bytes;
    assign led3_n = ~count[25];
    assign led4_n = ~saw_reset;
    assign led5_n = ~saw_dir;

endmodule
