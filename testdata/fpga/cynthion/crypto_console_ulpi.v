// crypto_console_ulpi — the crypto console behind a USB serial port, with
// the ULPI bus on the outside.
//
// What it does
//   `crypto_console` is the command interface and `usb_cdc_acm_ulpi` is the
//   serial port; this is the wire between them and nothing else.
//   `usb_crypto_console.v` adds the transceiver's three-state buffers, the
//   power-on reset and the six LEDs.
//
//   It exists as a module of its own for exactly the reason
//   `examples/mos6502_monitor/rtl/monitor_ulpi.v` does: so that **the whole
//   design can be driven through a transceiver model**. `tests/ip_library.rs`
//   has one, written from the ULPI specification and including the
//   behaviour the part on this board actually has — receive commands that
//   report LineState *late*, which ULPI 1.1 §3.8.1.3 forbids in so many
//   words and a Microchip USB3343 does anyway. That model's harness takes a
//   design whose ports are named the way `usb_device_ulpi`'s are, which is
//   what this port list is. So the simulation and the bitstream are the same
//   hierarchy, and the test drives the module the board contains rather than
//   a copy of its wiring.
//
// When the console is switched on, and why it is not at reset
//   The console is held in reset until the host has **configured** the port,
//   and put back into it by a bus reset, because `configured` goes low on
//   one. `crypto_console`'s reset state prints the help line, and a banner
//   printed into an endpoint that a bus reset then clears is a banner
//   nobody sees — which is the argument
//   `examples/mos6502_monitor/rtl/monitor_ulpi.v` makes about its prompt,
//   for the same stack and the same reason.
module crypto_console_ulpi #(
    parameter [15:0]  VID         = 16'h1209,
    parameter [15:0]  PID         = 16'h0001,
    parameter [6:0]   TURNAROUND  = 7'd9,
    // The Microchip USB3343 register that undoes this board's crossed
    // DP / DM pair. The board's register, not ULPI's.
    parameter [5:0]   VENDOR_ADDR = 6'h39,
    parameter [7:0]   VENDOR_DATA = 8'h06,
    // Bits of the SHA-256 message byte counter; `crypto_console`'s own
    // parameter and that module's header says why 32.
    parameter integer LEN_BITS    = 32,
    // Whether each crypto core is built. `crypto_console`'s own parameters,
    // and that module's header says why they exist: the design with both of
    // them does not route on this part, and the one with either of them
    // does.
    parameter integer WITH_HASH   = 1,
    parameter integer WITH_CIPHER = 1
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

    // What this port reports to the host as its line state.
    input  wire [6:0] serial_state,

    // For the board's LEDs, and so a test can see the cores rather than
    // infer them from the bytes that come back.
    output wire       hash_busy,
    output wire       cipher_active,
    output wire       saw_line,
    output wire       saw_answer
);
    // Named for what they carry rather than for the endpoint's port names,
    // for the reason `monitor_ulpi.v` gives: a harness that looks up
    // `out_data` at a design's top means the endpoint's interface brought
    // out for *it* to drive, and these are wires between two modules of
    // this design.
    wire [7:0] cmd_byte;
    wire       cmd_valid, cmd_room;
    wire [7:0] ans_byte;
    wire       ans_valid, ans_room, ans_flush;

    usb_cdc_acm_ulpi #(
        .VID         (VID),
        .PID         (PID),
        .TURNAROUND  (TURNAROUND),
        .VENDOR_ADDR (VENDOR_ADDR),
        .VENDOR_DATA (VENDOR_DATA)
    ) u_acm (
        .clk60        (clk60),
        .rst_n        (rst_n),
        .ulpi_data_i  (ulpi_data_i),
        .ulpi_data_o  (ulpi_data_o),
        .ulpi_data_oe (ulpi_data_oe),
        .ulpi_dir     (ulpi_dir),
        .ulpi_nxt     (ulpi_nxt),
        .ulpi_stp     (ulpi_stp),
        .ulpi_rst_n   (ulpi_rst_n),
        .address      (address),
        .configured   (configured),
        .usb_reset    (usb_reset),
        .phy_ready    (phy_ready),
        .out_data     (cmd_byte),
        .out_valid    (cmd_valid),
        // The console ends a message at a carriage return and not at a
        // packet boundary, so where the host chose to break its writes is
        // not something this design has any use for.
        .out_last     (),
        .out_ready    (cmd_room),
        .in_data      (ans_byte),
        .in_valid     (ans_valid),
        .in_ready     (ans_room),
        .in_commit    (ans_flush),
        // The line coding a host sets is accepted and ignored: there is no
        // serial line on the other side of this port, only logic, so there
        // is no rate for a divisor to be wrong about.
        // `testdata/fpga/cynthion/usb_cdc_uart.v` is the design that does
        // follow it, because it has a real 8N1 waveform on a pin.
        .baud         (),
        .char_format  (),
        .parity       (),
        .data_bits    (),
        .dtr          (),
        .rts          (),
        .serial_state (serial_state)
    );

    wire console_rst_n = rst_n & configured;

    crypto_console #(
        .LEN_BITS    (LEN_BITS),
        .WITH_HASH   (WITH_HASH),
        .WITH_CIPHER (WITH_CIPHER)
    ) u_console (
        .clk           (clk60),
        .rst_n         (console_rst_n),
        .rx_data       (cmd_byte),
        .rx_valid      (cmd_valid),
        .rx_ready      (cmd_room),
        .tx_data       (ans_byte),
        .tx_valid      (ans_valid),
        .tx_ready      (ans_room),
        .tx_commit     (ans_flush),
        .hash_busy     (hash_busy),
        .cipher_active (cipher_active),
        .saw_line      (saw_line),
        .saw_answer    (saw_answer)
    );
endmodule
