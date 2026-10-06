// usb_hub_ulpi — a USB hub on a board whose USB lines go through a ULPI
// transceiver: `usb_hub` behind `ip/usb_device_ulpi`'s link layer.
//
// What it does
//   This is to `usb_hub` what `usb_device_ulpi` is to `usb_dev_core`:
//   `usb_ulpi_link` does the ULPI bus — the turnaround, the transmit and
//   receive commands, the transceiver's registers — and the transceiver does
//   the line in silicon. One 60 MHz clock and no PLL, because ULPI's
//   interface clock rate is 60 MHz and that is what such a board's oscillator
//   already is.
//
//   It exists for the same board the rest of the ULPI blocks exist for: on a
//   Great Scott Gadgets Cynthion every USB port goes through its own
//   transceiver and the FPGA cannot drive the pair at all.
//   `ip/usb_device_ulpi/README.md` states the protocol with the provenance of
//   every fact.
//
//   **This is the wrapper that matters for this block**, because the port a
//   hub presents to a host and the port its own downstream device is on are
//   two different transceivers on the same die — the AUX port and the TARGET
//   port of that board. `testdata/fpga/cynthion/usb_hub_target.v` is the two
//   of them: this on AUX, `ip/usb_host_ulpi` on TARGET, and
//   `port_attached` wired from the one to the other.
//
//   Every port but the ULPI bus is `usb_hub`'s, and that module's header is
//   where the descriptors, the class requests and the status-change endpoint
//   are stated. `phy_ready` goes high when the transceiver has been reset,
//   put into full-speed peripheral mode and read back to confirm it.
//
// What it does not do
//   Everything `usb_hub` does not do, and `usb_ulpi_link`'s own limits: full
//   speed only, no suspend, no VBUS or ID handling, no high speed.
//
//   The eight data lines are split into `ulpi_data_i`, `ulpi_data_o` and
//   `ulpi_data_oe`, and `dir` turns them around; the three-state buffers
//   belong to the top of the design, as does forwarding the clock to the
//   transceiver's clock pin when the board asks the FPGA for it.
module usb_hub_ulpi #(
    parameter [15:0] VID          = 16'h1209,
    parameter [15:0] PID          = 16'h0001,
    // The link layer's own parameters; `usb_ulpi_link` and `usb_device_ulpi`
    // state each of them at length. `VENDOR_ADDR` and `VENDOR_DATA` are the
    // one register a board rather than ULPI can need written — on a Cynthion,
    // the bit that undoes a crossed DP / DM pair.
    parameter        RESET_CYCLES = 300,
    parameter        SE0_CYCLES   = 150,
    parameter        LINE_TRIES   = 40000,
    parameter [6:0]  TURNAROUND   = 7'd9,
    parameter [5:0]  VENDOR_ADDR  = 6'h00,
    parameter [7:0]  VENDOR_DATA  = 8'h00,
    parameter [7:0]  CFG_ATTR     = 8'h80,
    parameter [7:0]  CFG_POWER    = 8'd50,
    parameter [6:0]  MAXPKT0      = 7'd64,
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

    // What is on the downstream port, and what the host has made of it;
    // `usb_hub`'s own port comments say what each one is.
    input  wire       port_attached,
    input  wire       port_low_speed,
    output wire       port_power,
    output wire       port_enabled,
    output wire       port_suspended,
    // The port reset, as a level and a pulse: `usb_hub_req`'s "THE RESET,
    // WHICH NOW TAKES TIME" is the handshake, and tying `port_reset_done` high
    // makes the reset finish in the cycle it is asked for.
    output wire       port_reset,
    input  wire       port_reset_done
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

    usb_hub #(
        .VID        (VID),
        .PID        (PID),
        .CFG_ATTR   (CFG_ATTR),
        .CFG_POWER  (CFG_POWER),
        .MAXPKT0    (MAXPKT0),
        .BUF_RAM    (BUF_RAM),
        .TURNAROUND (TURNAROUND)
    ) u_hub (
        .clk            (clk60),
        .rst_n          (rst_n),
        .rx_data        (rx_data),
        .rx_valid       (rx_valid),
        .rx_eop         (rx_eop),
        .rx_active      (rx_active),
        .line_idle      (line_idle),
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
        .port_suspended  (port_suspended),
        .port_reset      (port_reset),
        .port_reset_done (port_reset_done)
    );

    assign usb_reset = bus_reset;
endmodule
