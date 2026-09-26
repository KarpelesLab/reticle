// usb_device_fs — a USB 2.0 full-speed device with a minimal control
// endpoint.
//
// What it does
//   Everything between the D+ / D- pins and a device a host can
//   enumerate: `usb_fs_rx` and `usb_fs_tx` for the line (NRZI, bit
//   stuffing, SYNC and EOP) and `usb_ctrl_ep` above them for the device
//   itself — packet decoding with the PID check nibble, the CRC5 of
//   tokens and the CRC16 of data packets checked, and endpoint 0
//   answering the standard requests a host needs to enumerate it. Runs
//   on a 48 MHz clock, four samples a bit; `usb_device_fs_pll` gets that
//   from the device's PLL and a 12 MHz board clock.
//
//   `usb_ctrl_ep` is a module of its own because `usb_device_ulpi` uses
//   it too: on a board whose USB lines go through a ULPI transceiver the
//   line-level half of this block has nothing to drive, and the device
//   above it is unchanged.
//
//   A packet whose PID check fails, whose CRC is wrong or which ends off
//   a byte boundary is ignored, as the specification asks — no
//   handshake, so the host retries. A token addressed elsewhere, or to
//   another endpoint, is ignored too.
//
//   Endpoint 0, maximum packet size 8:
//
//     GET_DESCRIPTOR, device        the 18-byte device descriptor, VID
//                                   and PID from the parameters
//     GET_DESCRIPTOR, configuration one configuration of one interface
//                                   with no endpoints, vendor class,
//                                   bus powered at 100 mA: 18 bytes
//     SET_ADDRESS                   taken after the status stage, as the
//                                   specification says
//     SET_CONFIGURATION 0 or 1      accepted; `configured` follows it
//
//   A descriptor goes out in as many DATA1 / DATA0 packets as it takes,
//   never more than the host's wLength, and a packet the host does not
//   acknowledge is sent again with the same toggle. The status stage is
//   an OUT of zero length after a read and an IN of zero length after
//   the others. Anything else — string descriptors, GET_STATUS,
//   requests to an interface or an endpoint, class and vendor requests —
//   is answered with STALL until the next SETUP.
//
//   A bus reset (SE0 for more than 2.5 us) sets the address back to 0
//   and the configuration to none. `usb_dp_pu` asks for the 1.5 kOhm
//   pull-up on D+ that tells the host a full-speed device is attached;
//   it is high once the block is out of reset.
//
//   The device answers a token or a data packet two bit times after the
//   host's EOP has ended, inside the 6.5 bit times the specification
//   allows.
//
// What it does not do
//   Endpoint 0 only: no bulk, interrupt or isochronous endpoints, so
//   there is nothing to move data with once enumerated. It is the part
//   every USB function needs first and the part that is easiest to get
//   wrong, and it is what the other endpoints would be added beside.
//
//   No strings, no remote wake-up, no suspend (a device must draw under
//   2.5 mA after 3 ms of idle, which is a board's business), no
//   SOF tracking and no low speed. No high speed, which needs a 480
//   Mbit/s transceiver no FPGA IO provides.
//
//   The pins are split, as every bidirectional pin in this library is:
//   `usb_dp_o`, `usb_dn_o` and `usb_oe` out, `usb_dp_i` and `usb_dn_i`
//   in, and the three-state buffers at the top of the design.
module usb_device_fs #(
    parameter [15:0] VID = 16'h1209,
    parameter [15:0] PID = 16'h0001
) (
    input  wire       clk48,
    input  wire       rst_n,

    input  wire       usb_dp_i,
    input  wire       usb_dn_i,
    output wire       usb_dp_o,
    output wire       usb_dn_o,
    output wire       usb_oe,
    output wire       usb_dp_pu,

    output wire [6:0] address,
    output wire       configured,
    output wire       usb_reset
);
    // The line: NRZI, bit stuffing, SYNC and EOP, four samples a bit.
    wire       tx_busy;
    wire [7:0] rx_data;
    wire       rx_valid, rx_eop, rx_error, rx_active, rx_idle_j, bus_reset;

    usb_fs_rx u_rx (
        .clk       (clk48),
        .rst_n     (rst_n),
        .dp        (usb_dp_i),
        .dn        (usb_dn_i),
        .enable    (~tx_busy),
        .data      (rx_data),
        .valid     (rx_valid),
        .eop       (rx_eop),
        .error     (rx_error),
        .active    (rx_active),
        .idle_j    (rx_idle_j),
        .bus_reset (bus_reset)
    );

    wire       tx_start;
    wire [3:0] tx_pid;
    wire       tx_with_data;
    wire [3:0] tx_len;
    wire [3:0] tx_index;
    wire [7:0] tx_byte;

    usb_fs_tx u_tx (
        .clk       (clk48),
        .rst_n     (rst_n),
        .start     (tx_start),
        .pid       (tx_pid),
        .with_data (tx_with_data),
        .len       (tx_len),
        .index     (tx_index),
        .byte_in   (tx_byte),
        .busy      (tx_busy),
        .dp        (usb_dp_o),
        .dn        (usb_dn_o),
        .oe        (usb_oe)
    );

    // The device. Two bit times of idle J after the host's EOP before an
    // answer starts, which is eight cycles of this clock.
    usb_ctrl_ep #(
        .VID        (VID),
        .PID        (PID),
        .TURNAROUND (4'd8)
    ) u_ep (
        .clk          (clk48),
        .rst_n        (rst_n),
        .rx_data      (rx_data),
        .rx_valid     (rx_valid),
        .rx_eop       (rx_eop),
        .rx_active    (rx_active),
        .line_idle    (rx_idle_j),
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

    // The 1.5 kOhm pull-up on D+ says a full-speed device is attached.
    assign usb_dp_pu = 1'b1;
    assign usb_reset = bus_reset;
endmodule
