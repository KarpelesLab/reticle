// monitor_ulpi — the machine behind a USB serial port, with the ULPI bus
// on the outside.
//
// What it does
//   `monitor_machine` is the computer and `usb_cdc_acm_ulpi` is the
//   serial port; this is the wire between them and nothing else. The
//   board's file adds the transceiver's three-state buffers, the
//   power-on reset, the LEDs and the waveform on ball C11.
//
//   It exists as a module of its own so that **the whole machine can be
//   driven through a transceiver model**. `tests/ip_library.rs` has one,
//   written from the ULPI specification, including the behaviour the
//   part on this board actually has — receive commands that report
//   LineState *late*, which ULPI 1.1 §3.8.1.3 forbids in so many words
//   and a Microchip USB3343 does anyway. That model's harness takes a
//   design whose ports are named the way `usb_device_ulpi`'s are, which
//   is what this file's port list is: `clk60`, the three-way split of
//   the data lines, and `address` / `configured` / `usb_reset` /
//   `phy_ready` brought out.
//
//   So the simulation and the bitstream are the same hierarchy, and the
//   test drives the module the board contains rather than a copy of its
//   wiring.
module monitor_ulpi #(
    parameter [15:0]  VID       = 16'h1209,
    parameter [15:0]  PID       = 16'h0001,
    parameter [6:0]   TURNAROUND = 7'd9,
    // The Microchip USB3343 register that undoes this board's crossed
    // DP/DM pair. The board's register, not ULPI's.
    parameter [5:0]   VENDOR_ADDR = 6'h39,
    parameter [7:0]   VENDOR_DATA = 8'h06,
    // Clocks per 6502 bus cycle, and bytes of RAM.
    parameter integer CPU_DIV    = 59,
    parameter integer RAM_BYTES  = 4096,
    parameter integer FLUSH_CLKS = 16384
) (
    input  wire        clk60,
    input  wire        rst_n,

    // The ULPI bus to the transceiver.
    input  wire [7:0]  ulpi_data_i,
    output wire [7:0]  ulpi_data_o,
    output wire        ulpi_data_oe,
    input  wire        ulpi_dir,
    input  wire        ulpi_nxt,
    output wire        ulpi_stp,
    output wire        ulpi_rst_n,

    output wire [6:0]  address,
    output wire        configured,
    output wire        usb_reset,
    output wire        phy_ready,

    // What this port reports to the host as its line state.
    input  wire [6:0]  serial_state,

    // What the ACIA is programmed to, and every byte the processor
    // prints, for the board's waveform on a pin.
    output wire [31:0] acia_rate,
    output wire [7:0]  acia_control,
    output wire [7:0]  print_data,
    output wire        print_valid,
    // A byte the host sent reached the ACIA, for an LED that says so.
    output wire        key_taken
);
    // Named for what they carry rather than for the endpoint's own port
    // names. A test harness that looks up `out_data` at a design's top
    // means the endpoint's interface brought out for *it* to drive; these
    // are the wires between two modules of this design, and a name they
    // share with that convention would invite something to drive them.
    wire [7:0]  key_byte;
    wire        key_valid, key_room;
    wire [7:0]  print_byte;
    wire        print_strobe, print_room, print_flush;
    wire [31:0] host_rate;

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
        .out_data     (key_byte),
        .out_valid    (key_valid),
        .out_last     (),
        .out_ready    (key_room),
        .in_data      (print_byte),
        .in_valid     (print_strobe),
        .in_ready     (print_room),
        .in_commit    (print_flush),
        .baud         (host_rate),
        .char_format  (),
        .parity       (),
        .data_bits    (),
        .dtr          (),
        .rts          (),
        .serial_state (serial_state)
    );

    assign key_taken = key_valid & key_room;

    // -----------------------------------------------------------------
    // WHEN THE MACHINE IS SWITCHED ON, AND WHY IT IS NOT AT RESET
    // -----------------------------------------------------------------
    //
    // The 6502 is held in reset until the host has **configured** the
    // port, and put back into it by a bus reset, because `configured`
    // goes low on one.
    //
    // It is not a nicety. The monitor prints its prompt about four
    // milliseconds after it starts, and enumeration takes longer than
    // that — so a machine released at power-on prints `\` into an
    // endpoint that a bus reset then clears, and the first thing a person
    // sees when they open the port is nothing at all. Holding it until
    // the port exists means the prompt is produced into a live endpoint,
    // buffered there, and delivered the moment a terminal reads.
    //
    // It also matches what the machine is: a computer whose power comes
    // from the port. Unplugging it and plugging it in again is a cold
    // start, which is what it looks like from the other end too.
    wire machine_rst_n = rst_n & configured;

    monitor_machine #(
        .CPU_DIV    (CPU_DIV),
        .RAM_BYTES  (RAM_BYTES),
        .FLUSH_CLKS (FLUSH_CLKS)
    ) u_machine (
        .clk          (clk60),
        .rst_n        (machine_rst_n),
        .out_data     (key_byte),
        .out_valid    (key_valid),
        .out_ready    (key_room),
        .in_data      (print_byte),
        .in_valid     (print_strobe),
        .in_ready     (print_room),
        .in_commit    (print_flush),
        .host_rate    (host_rate),
        .acia_rate    (acia_rate),
        .acia_control (acia_control),
        .print_data   (print_data),
        .print_valid  (print_valid)
    );
endmodule
