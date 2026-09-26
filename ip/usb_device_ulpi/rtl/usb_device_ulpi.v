// usb_device_ulpi — a USB 2.0 full-speed device behind a ULPI
// transceiver.
//
// What it does
//   `usb_ulpi_link` for the ULPI bus and `usb_ctrl_ep` for the device:
//   the transceiver does NRZI, bit stuffing, SYNC, the EOP and the line
//   itself, and this block does the bytes — packet decoding with the PID
//   check nibble, the CRC5 of tokens and the CRC16 of data packets
//   checked and generated, and endpoint 0 answering the standard
//   requests a host needs to enumerate it.
//
//   `usb_ctrl_ep` is the same module `usb_device_fs` uses, reached
//   through a `depends` line rather than copied, so there is one
//   statement of the control endpoint for both cores. What differs is
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
//   Full speed only, endpoint 0 only, no suspend, no VBUS or ID
//   handling, and no high speed. `usb_ulpi_link` and `usb_ctrl_ep` each
//   say what they leave out, and `README.md` in this package says what
//   of ULPI is implemented, what is not, and how sure of each fact this
//   is.
//
//   Nothing here has run on a board. It enumerates in simulation,
//   against a transceiver model written from the specification, with a
//   USB host model on the other side of that model sending real packets.
//   No host has seen it.
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
    parameter        SE0_CYCLES   = 150
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
    output wire       phy_ready
);
    wire [7:0] rx_data;
    wire       rx_valid, rx_eop, rx_active, line_idle, bus_reset;
    wire       tx_start, tx_with_data, tx_busy;
    wire [3:0] tx_pid, tx_len, tx_index;
    wire [7:0] tx_byte;

    usb_ulpi_link #(
        .RESET_CYCLES (RESET_CYCLES),
        .SE0_CYCLES   (SE0_CYCLES)
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
    usb_ctrl_ep #(
        .VID        (VID),
        .PID        (PID),
        .TURNAROUND (4'd9)
    ) u_ep (
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
        .configured   (configured)
    );

    assign usb_reset = bus_reset;
endmodule
