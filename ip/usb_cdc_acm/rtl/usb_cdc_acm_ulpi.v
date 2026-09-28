// usb_cdc_acm_ulpi — a USB serial port on a board whose USB lines go
// through a ULPI transceiver: `usb_cdc_acm` behind
// `ip/usb_device_ulpi`'s link layer.
//
// What it does
//   This is to `usb_cdc_acm` what `usb_device_ulpi` is to `usb_dev_core`:
//   `usb_ulpi_link` does the ULPI bus — the turnaround, the transmit and
//   receive commands, the transceiver's registers — and the transceiver
//   does the line in silicon. One 60 MHz clock and no PLL, because ULPI's
//   interface clock rate is 60 MHz and that is what such a board's
//   oscillator already is.
//
//   It exists for the same board `usb_device_ulpi` exists for: on a Great
//   Scott Gadgets Cynthion every USB port goes through its own transceiver
//   and the FPGA cannot drive the pair at all. `ip/usb_device_ulpi`'s
//   README.md states the protocol, with the provenance of every fact.
//
//   Every port but the ULPI bus is `usb_cdc_acm`'s, and that module's
//   header is where the descriptors, the class requests and the
//   notification endpoint are stated. `phy_ready` goes high when the
//   transceiver has been reset, put into full-speed peripheral mode and
//   read back to confirm it.
//
// What it does not do
//   Everything `usb_cdc_acm` does not do, and `usb_ulpi_link`'s own
//   limits: full speed only, no suspend, no VBUS or ID handling, no high
//   speed.
//
//   The eight data lines are split into `ulpi_data_i`, `ulpi_data_o` and
//   `ulpi_data_oe`, and `dir` turns them around; the three-state buffers
//   belong to the top of the design, as does forwarding the clock to the
//   transceiver's clock pin when the board asks the FPGA for it.
module usb_cdc_acm_ulpi #(
    parameter [15:0] VID          = 16'h1209,
    parameter [15:0] PID          = 16'h0001,
    // The link layer's own parameters; `usb_ulpi_link` and
    // `usb_device_ulpi` state each of them at length. `VENDOR_ADDR` and
    // `VENDOR_DATA` are the one register a board rather than ULPI can need
    // written — on a Cynthion, the bit that undoes a crossed DP / DM pair.
    parameter        RESET_CYCLES = 300,
    parameter        SE0_CYCLES   = 150,
    parameter        LINE_TRIES   = 40000,
    parameter [6:0]  TURNAROUND   = 7'd9,
    parameter [5:0]  VENDOR_ADDR  = 6'h00,
    parameter [7:0]  VENDOR_DATA  = 8'h00,
    parameter [7:0]  CFG_ATTR     = 8'h80,
    parameter [7:0]  CFG_POWER    = 8'd50,
    // Bytes in a bulk packet, and in an endpoint 0 packet; `usb_cdc_acm`'s
    // parameters of the same names say which values USB 2.0 allows.
    parameter [6:0]  MAXPKT       = 7'd64,
    parameter [6:0]  MAXPKT0      = 7'd64,
    // What shape the bulk endpoints' packet buffers take: 1 an array, which is
    // a distributed RAM on a family that has one, 0 a shift register.
    // `usb_bulk_ep`'s parameter of the same name has the measurements.
    parameter        BUF_RAM      = 1
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
    output wire        rts,

    // What this port reports to the host as its line state, straight into
    // `usb_cdc_acm`, whose own port comment names every bit and says what a
    // design with no modem lines ties it to.
    input  wire [6:0]  serial_state
);
    wire [7:0] rx_data;
    wire       rx_valid, rx_eop, rx_active, line_idle, bus_reset;
    wire       tx_start, tx_with_data, tx_busy;
    wire [3:0] tx_pid;
    wire [6:0] tx_len, tx_index;
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

    usb_cdc_acm #(
        .VID        (VID),
        .PID        (PID),
        .CFG_ATTR   (CFG_ATTR),
        .CFG_POWER  (CFG_POWER),
        .MAXPKT     (MAXPKT),
        .MAXPKT0    (MAXPKT0),
        .BUF_RAM    (BUF_RAM),
        .TURNAROUND (TURNAROUND)
    ) u_acm (
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

    assign usb_reset = bus_reset;
endmodule
