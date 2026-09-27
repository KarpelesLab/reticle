// usb_device_ulpi — a USB 2.0 full-speed device behind a ULPI
// transceiver.
//
// What it does
//   `usb_ulpi_link` for the ULPI bus and `usb_dev_core` for the device:
//   the transceiver does NRZI, bit stuffing, SYNC, the EOP and the line
//   itself, and this block does the bytes — packet decoding with the PID
//   check nibble, the CRC5 of tokens and the CRC16 of data packets
//   checked and generated, endpoint 0 answering the standard requests a
//   host needs to enumerate it, and a bulk endpoint pair whose byte
//   interface comes out of this block.
//
//   `usb_dev_core` is the same module `usb_device_fs` uses, reached
//   through a `depends` line rather than copied, so there is one
//   statement of the device for both cores. What differs is
//   everything below it, and the reason for a second core is a board:
//   all three USB ports of a Great Scott Gadgets Cynthion go through
//   ULPI transceivers, and its FPGA cannot reach the data lines at all —
//   the two pins wired to them are inputs, for sniffing. An encoder and
//   a serialiser have nothing to drive there. A byte-parallel bus to a
//   transceiver is what such a board wants.
//
//   Clocking. One clock, 60 MHz, and no PLL: ULPI's own clock rate is
//   60 MHz and a Cynthion's oscillator is 60 MHz, so the board's clock
//   is the interface's clock. The transceivers there are in ULPI's input
//   clock mode — `clk_dir='o'` in the platform description, the FPGA
//   driving the clock to the transceiver — so the design's top forwards
//   this same clock to the transceiver's clock pin. That is the one
//   thing this block does not do for a design: a clock leaves an FPGA
//   through a dedicated output, not through a port of an IP block. There
//   is no `usb_device_ulpi_pll` for the same reason there is a
//   `usb_device_fs_pll`: nothing needs making.
//
//   `phy_ready` goes high when the transceiver has been reset, put into
//   full-speed peripheral mode, and read back to confirm it. Until then
//   the device answers nothing: it does not even look at the bus, since a
//   transceiver being reset drives it with whatever it likes. The pull-up
//   on D+ that tells a host a device is attached — `usb_device_fs`'s
//   `usb_dp_pu`, a register bit here rather than a pin — comes on with
//   the first of those register writes, a few microseconds before
//   `phy_ready`, so a host cannot see this device long before it can
//   answer.
//
//   `address`, `configured` and `usb_reset` are what the FS core reports:
//   the address the host assigned, whether SET_CONFIGURATION has been
//   accepted, and whether the host is holding the bus in reset.
//
// What it does not do
//   Full speed only, one bulk endpoint pair, no suspend, no VBUS or ID
//   handling, and no high speed. `usb_ulpi_link`, `usb_dev_core` and
//   `usb_bulk_ep` each say what they leave out, and `README.md` in this
//   package says what of ULPI is implemented, what is not, and how sure of
//   each fact this is.
//
//   THE BYTES. `out_*` is what the host sent to the data endpoint's OUT,
//   a byte at a time with `out_last` on the last byte of each packet;
//   `in_*` is what its IN will send, and `in_commit` sends what has been
//   given however short it is. `usb_bulk_ep` states the whole contract.
//
//   The eight data lines are split into `ulpi_data_i`, `ulpi_data_o` and
//   `ulpi_data_oe`, as every bidirectional pin in this library is, and
//   `dir` turns them around. The three-state buffers belong to the top
//   of the design. `ulpi_rst_n` is active low, which is what the
//   transceiver's own reset pin is; it is not a ULPI signal at all.
module usb_device_ulpi #(
    parameter [15:0] VID          = 16'h1209,
    parameter [15:0] PID          = 16'h0001,
    // Cycles the transceiver's reset is held, and the idle bus waited
    // for afterwards: 5 us at 60 MHz.
    parameter        RESET_CYCLES = 300,
    // Cycles of SE0 that make a bus reset: 2.5 us at 60 MHz.
    parameter        SE0_CYCLES   = 150,
    // Attempts at the start-up's LineState read while it still says SE0,
    // which a pair whose pull-up has just been connected does for
    // milliseconds. `usb_ulpi_link` says what was measured.
    parameter        LINE_TRIES   = 40000,
    // One register of the transceiver's own, written before anything a
    // host can see and read back afterwards; `6'h00` is none, which is
    // the default. `usb_ulpi_link`'s own parameter says what a board can
    // need one for — the short version is that a board may exchange DP
    // and DM between the transceiver and its connector, and then a bit in
    // a register ULPI does not describe is the only thing that can undo
    // it.
    // Cycles of an idle bus at J, after the end of a received packet, before
    // the answer's transmit command goes out. ULPI 1.1 Table 10 allows a
    // full-speed Link **7 to 18** interface clocks, counted from the receive
    // command that reports LineState's SE0-to-J transition, and says the
    // window "ensure[s] inter-packet delays of 2-6.5 bit times", which is
    // USB 2.0's own requirement. Nine puts the transmit command eleven or
    // twelve clocks after that receive command, in the middle of it.
    parameter [6:0]  TURNAROUND   = 7'd9,
    parameter [5:0]  VENDOR_ADDR  = 6'h00,
    parameter [7:0]  VENDOR_DATA  = 8'h00,
    parameter [7:0]  DEV_CLASS    = 8'hFF,
    parameter [7:0]  DEV_SUBCLASS = 8'h00,
    parameter [7:0]  DEV_PROTOCOL = 8'h00,
    parameter [7:0]  CFG_ATTR     = 8'h80,
    parameter [7:0]  CFG_POWER    = 8'd50,
    // The class's interface and endpoint descriptors, in descriptor order,
    // and their length in bytes; `usb_ctrl_ep` says what is derived from
    // them and what is not.
    parameter integer IFACE_BYTES = 23,
    parameter [IFACE_BYTES*8-1:0] IFACE_DESC = {
        8'd9, 8'd4, 8'd0, 8'd0, 8'd2, 8'hFF, 8'h00, 8'h00, 8'd0,
        8'd7, 8'd5, 8'h01, 8'd2, 8'd8, 8'd0, 8'd0,
        8'd7, 8'd5, 8'h81, 8'd2, 8'd8, 8'd0, 8'd0
    },
    parameter [3:0]  DATA_ENDP    = 4'd1,
    parameter [3:0]  MAXPKT       = 4'd8
) (
    input  wire       clk60,
    input  wire       rst_n,

    // The ULPI bus to the transceiver.
    input  wire [7:0] ulpi_data_i,
    output wire [7:0] ulpi_data_o,
    output wire       ulpi_data_oe,
    input  wire       ulpi_dir,
    input  wire       ulpi_nxt,
    output wire       ulpi_stp,
    output wire       ulpi_rst_n,

    output wire [6:0] address,
    output wire       configured,
    output wire       usb_reset,
    output wire       phy_ready,

    // The data endpoint's bytes.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit
);
    wire [7:0] rx_data;
    wire       rx_valid, rx_eop, rx_active, line_idle, bus_reset;
    wire       tx_start, tx_with_data, tx_busy;
    wire [3:0] tx_pid, tx_len, tx_index;
    wire [7:0] tx_byte;

    usb_ulpi_link #(
        .RESET_CYCLES (RESET_CYCLES),
        .SE0_CYCLES   (SE0_CYCLES),
        .LINE_TRIES   (LINE_TRIES),
        .VENDOR_ADDR  (VENDOR_ADDR),
        .VENDOR_DATA  (VENDOR_DATA)
    ) u_link (
        .clk          (clk60),
        .rst_n        (rst_n),
        .ulpi_dir     (ulpi_dir),
        .ulpi_nxt     (ulpi_nxt),
        .ulpi_data_i  (ulpi_data_i),
        .ulpi_data_o  (ulpi_data_o),
        .ulpi_data_oe (ulpi_data_oe),
        .ulpi_stp     (ulpi_stp),
        .ulpi_rst_n   (ulpi_rst_n),
        .rx_data      (rx_data),
        .rx_valid     (rx_valid),
        .rx_eop       (rx_eop),
        .rx_active    (rx_active),
        .line_idle    (line_idle),
        .bus_reset    (bus_reset),
        .tx_start     (tx_start),
        .tx_pid       (tx_pid),
        .tx_with_data (tx_with_data),
        .tx_len       (tx_len),
        .tx_index     (tx_index),
        .tx_byte      (tx_byte),
        .tx_busy      (tx_busy),
        .phy_ready    (phy_ready)
    );

    // ULPI states the delay between a received packet and the answer as
    // a count of interface clocks: 7 to 18 for a full-speed link,
    // counted from the receive command that reports the bus back at J
    // (ULPI 1.1 Table 10). Nine cycles of `line_idle` here puts the
    // transmit command on the bus eleven clocks after it, in the middle
    // of that window, which is the 2 to 6.5 bit times USB asks for.
    usb_dev_core #(
        .VID          (VID),
        .PID          (PID),
        .DEV_CLASS    (DEV_CLASS),
        .DEV_SUBCLASS (DEV_SUBCLASS),
        .DEV_PROTOCOL (DEV_PROTOCOL),
        .CFG_ATTR     (CFG_ATTR),
        .CFG_POWER    (CFG_POWER),
        .IFACE_BYTES  (IFACE_BYTES),
        .IFACE_DESC   (IFACE_DESC),
        .DATA_ENDP    (DATA_ENDP),
        .MAXPKT       (MAXPKT),
        .TURNAROUND   (TURNAROUND)
    ) u_dev (
        .clk          (clk60),
        .rst_n        (rst_n),
        .rx_data      (rx_data),
        .rx_valid     (rx_valid),
        .rx_eop       (rx_eop),
        .rx_active    (rx_active),
        .line_idle    (line_idle),
        .bus_reset    (bus_reset),
        .tx_start     (tx_start),
        .tx_pid       (tx_pid),
        .tx_with_data (tx_with_data),
        .tx_len       (tx_len),
        .tx_index     (tx_index),
        .tx_byte      (tx_byte),
        .tx_busy      (tx_busy),
        .address      (address),
        .configured   (configured),
        // No class layer: endpoint 0 stalls what it does not itself
        // implement, which is what `class_claim` low means, and
        // `usb_ctrl_ep`'s hook costs nothing when it is tied off. A device
        // with a class is a block above this one; `ip/usb_cdc_acm` is the
        // first.
        .class_claim  (1'b0),
        .class_len    (7'd0),
        .class_byte   (8'd0),
        // ... and no second endpoint, which `NOTIF_ENDP`'s default of zero
        // already says.
        .notif_data   (8'd0),
        .notif_valid  (1'b0),
        .notif_commit (1'b0),
        .out_data     (out_data),
        .out_valid    (out_valid),
        .out_last     (out_last),
        .out_ready    (out_ready),
        .in_data      (in_data),
        .in_valid     (in_valid),
        .in_ready     (in_ready),
        .in_commit    (in_commit)
    );

    assign usb_reset = bus_reset;
endmodule
