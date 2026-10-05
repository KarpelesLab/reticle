// usb_hub_fs — a USB hub on a board whose D+ / D- reach the FPGA:
// `usb_hub` behind `ip/usb_device_fs`'s own line layer.
//
// What it does
//   This is to `usb_hub` what `usb_device_fs` is to `usb_dev_core`:
//   `usb_fs_rx` and `usb_fs_tx` do the line — NRZI, bit stuffing, SYNC and
//   the EOP, four samples a bit on a 48 MHz clock — and the class layer above
//   them is unchanged. `usb_dp_pu` asks for the 1.5 kOhm pull-up on D+ that
//   tells a host a full-speed device is attached.
//
//   **No board in this project has this wrapper on it**, and it is here for
//   two reasons rather than one. The first is the library's own symmetry:
//   every class layer here has both link layers, because the class is the same
//   either way and that is the argument for `usb_dev_core` existing at all.
//   The second is that the two link layers fail differently — the full-speed
//   one has a bit clock recovered from the pair and the ULPI one has a
//   transceiver with turnaround rules — so a class tested through both is
//   tested against something, and `tests/ip_library.rs` runs every hub
//   assertion through each.
//
//   Every port but the four the line needs is `usb_hub`'s, and that module's
//   header is where the descriptors, the class requests and the status-change
//   endpoint are stated.
//
// What it does not do
//   Everything `usb_hub` does not do, and the line layer's own limits: full
//   speed only, no low speed, no high speed.
//
//   The pins are split, as every bidirectional pin in this library is:
//   `usb_dp_o`, `usb_dn_o` and `usb_oe` out, `usb_dp_i` and `usb_dn_i` in,
//   and the three-state buffers at the top of the design.
module usb_hub_fs #(
    parameter [15:0] VID       = 16'h1209,
    parameter [15:0] PID       = 16'h0001,
    parameter [7:0]  CFG_ATTR  = 8'h80,
    parameter [7:0]  CFG_POWER = 8'd50,
    parameter [6:0]  MAXPKT0   = 7'd64,
    parameter        BUF_RAM   = 1
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
    output wire       usb_reset,

    // What is on the downstream port, and what the host has made of it;
    // `usb_hub`'s own port comments say what each one is.
    input  wire       port_attached,
    input  wire       port_low_speed,
    output wire       port_power,
    output wire       port_enabled,
    output wire       port_suspended,
    output wire       port_reset
);
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
    wire [6:0] tx_len;
    wire [6:0] tx_index;
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

    // Two bit times of idle J after the host's EOP before an answer starts,
    // which is eight cycles of this clock — the same number `usb_device_fs`
    // gives its core.
    usb_hub #(
        .VID        (VID),
        .PID        (PID),
        .CFG_ATTR   (CFG_ATTR),
        .CFG_POWER  (CFG_POWER),
        .MAXPKT0    (MAXPKT0),
        .BUF_RAM    (BUF_RAM),
        .TURNAROUND (7'd8)
    ) u_hub (
        .clk            (clk48),
        .rst_n          (rst_n),
        .rx_data        (rx_data),
        .rx_valid       (rx_valid),
        .rx_eop         (rx_eop),
        .rx_active      (rx_active),
        .line_idle      (rx_idle_j),
        .bus_reset      (bus_reset),
        .tx_start       (tx_start),
        .tx_pid         (tx_pid),
        .tx_with_data   (tx_with_data),
        .tx_len         (tx_len),
        .tx_index       (tx_index),
        .tx_byte        (tx_byte),
        .tx_busy        (tx_busy),
        .address        (address),
        .configured     (configured),
        .port_attached  (port_attached),
        .port_low_speed (port_low_speed),
        .port_power     (port_power),
        .port_enabled   (port_enabled),
        .port_suspended (port_suspended),
        .port_reset     (port_reset)
    );

    assign usb_dp_pu = 1'b1;
    assign usb_reset = bus_reset;
endmodule
