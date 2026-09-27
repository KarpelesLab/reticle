// usb_cdc_acm_fs — a USB serial port on a board whose D+ / D- reach the
// FPGA: `usb_cdc_acm` behind `ip/usb_device_fs`'s own line layer.
//
// What it does
//   This is to `usb_cdc_acm` what `usb_device_fs` is to `usb_dev_core`:
//   `usb_fs_rx` and `usb_fs_tx` do the line — NRZI, bit stuffing, SYNC and
//   the EOP, four samples a bit on a 48 MHz clock — and the class layer
//   above them is unchanged. A design that wants a serial port and has the
//   pair on two pins instantiates this and nothing else.
//
//   Every port but the four the line needs is `usb_cdc_acm`'s, and that
//   module's header is where the descriptors, the class requests and the
//   notification endpoint are stated. `usb_dp_pu` asks for the 1.5 kOhm
//   pull-up on D+ that tells a host a full-speed device is attached.
//
//   The 48 MHz is the caller's. There is no `usb_cdc_acm_fs_pll` beside
//   this: `usb_device_fs_pll` already exists for the plain device, and a
//   serial port on a 12 MHz board is that wrapper's PLL and this module,
//   which is two instantiations rather than a third package.
//
// What it does not do
//   Everything `usb_cdc_acm` does not do, and the line layer's own limits:
//   full speed only, no low speed, no high speed. `usb_fs_rx` and
//   `usb_fs_tx` state theirs.
//
//   The pins are split, as every bidirectional pin in this library is:
//   `usb_dp_o`, `usb_dn_o` and `usb_oe` out, `usb_dp_i` and `usb_dn_i` in,
//   and the three-state buffers at the top of the design.
module usb_cdc_acm_fs #(
    parameter [15:0] VID       = 16'h1209,
    parameter [15:0] PID       = 16'h0001,
    parameter [7:0]  CFG_ATTR  = 8'h80,
    parameter [7:0]  CFG_POWER = 8'd50
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

    // The serial port's bytes.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit,

    // What the host asked the line to be, and the control lines.
    output wire [31:0] baud,
    output wire [7:0]  char_format,
    output wire [7:0]  parity,
    output wire [7:0]  data_bits,
    output wire        dtr,
    output wire        rts
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

    // Two bit times of idle J after the host's EOP before an answer starts,
    // which is eight cycles of this clock — the same number
    // `usb_device_fs` gives its core.
    usb_cdc_acm #(
        .VID        (VID),
        .PID        (PID),
        .CFG_ATTR   (CFG_ATTR),
        .CFG_POWER  (CFG_POWER),
        .TURNAROUND (7'd8)
    ) u_acm (
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
        .configured   (configured),
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

    assign usb_dp_pu = 1'b1;
    assign usb_reset = bus_reset;
endmodule
