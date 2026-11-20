// usb_cdc_vendor_fs — the composite CDC + vendor device (`usb_cdc_vendor`)
// behind `ip/usb/usb_device_fs`'s own full-speed line layer, for simulation.
//
// The Cynthion build uses `usb_cdc_vendor_ulpi`; this FS wrapper exists so the
// `tests/ip_library.rs` USB host model can drive the composite device over
// `usb_fs_rx`/`usb_fs_tx` and check the descriptors and the "smartcard-ctl"
// interface string, the same way `usb_cdc_acm_fs` lets it check a plain serial
// port. The vendor bulk interface is declared in the descriptors but tied
// inert here — the string descriptor it carries is answered on endpoint 0, so
// the control path is all this needs to exercise.
module usb_cdc_vendor_fs #(
    parameter [15:0] VID       = 16'h1209,
    parameter [15:0] PID       = 16'h0001,
    parameter [7:0]  CFG_ATTR  = 8'h80,
    parameter [7:0]  CFG_POWER = 8'd50,
    parameter [6:0]  MAXPKT    = 7'd64,
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

    // The CDC serial port's bytes (the card stream on the real device).
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit,

    output wire [31:0] baud,
    output wire [7:0]  char_format,
    output wire [7:0]  parity,
    output wire [7:0]  data_bits,
    output wire        dtr,
    output wire        rts,
    input  wire [6:0]  serial_state
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

    usb_cdc_vendor #(
        .VID        (VID),
        .PID        (PID),
        .CFG_ATTR   (CFG_ATTR),
        .CFG_POWER  (CFG_POWER),
        .MAXPKT     (MAXPKT),
        .MAXPKT0    (MAXPKT0),
        .BUF_RAM    (BUF_RAM),
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
        .rts          (rts),
        .serial_state (serial_state),
        // The vendor interface is enumerated but idle in this fixture.
        .vout_data    (),
        .vout_valid   (),
        .vout_last    (),
        .vout_ready   (1'b1),
        .vin_data     (8'd0),
        .vin_valid    (1'b0),
        .vin_ready    (),
        .vin_commit   (1'b0)
    );

    assign usb_dp_pu = 1'b1;
    assign usb_reset = bus_reset;
endmodule
