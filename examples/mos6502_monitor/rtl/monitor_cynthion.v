// monitor_cynthion — the machine on a Great Scott Gadgets Cynthion, with
// its console on a USB serial port.
//
// What it does
//   `monitor_machine` is a 6502 with an ACIA; `ip/usb/usb_cdc_acm`'s ULPI
//   form is a USB serial port; this file is the wire between them, plus
//   the board's transceiver bus and its six LEDs. Plug the board in,
//   open `/dev/ttyACM1`, and the `\` is the processor's.
//
//   One clock domain, 60 MHz, no PLL: ULPI's interface clock rate is
//   60 MHz and that is what this board's oscillator already is. The
//   board asks the FPGA for that clock back on ball D16, which is what
//   `assign ulpi_clk = clk` is.
//
//   `testdata/fpga/cynthion/usb_cdc_uart.v` is the same USB half of this
//   design with a UART where the computer is, and its header carries the
//   provenance of every ULPI ball, every LED pin and every safety
//   argument about this board. The short form: the data lines are
//   released whenever `ulpi_dir` is high, which is the bus's own
//   arbitration (ULPI 1.1 3.3); every direction is the one Great Scott
//   Gadgets' platform file gives it; the Type-C controllers, the VBUS
//   switches and the pseudo-supply pins are left alone; and the CONTROL
//   port the Apollo debugger lives on is a different transceiver on
//   different balls and is not mentioned here, so loading this design
//   cannot take the debugger away.
//
// The LEDs
//
//     LED 0   THE RATE CAME FROM THE HOST     `baud_ok`, live
//     LED 1   THE HOST CONFIGURED IT          `configured`, latched
//     LED 2   THE PROCESSOR PRINTED SOMETHING latched
//     LED 3   heartbeat, 0.89 Hz              the clock runs
//     LED 4   A KEY REACHED THE ACIA          latched
//     LED 5   THE MIRROR'S TRANSMIT LINE      live, dark while idle
//
//   Read from the bottom up:
//
//   * **All dark** — no clock, or the bitstream did not load.
//   * **LED 3 alone** — the fabric is running and the host has not
//     enumerated the device. That is also what a transceiver that never
//     came up looks like; `testdata/fpga/cynthion/usb_ulpi_device.v`
//     reports `phy_ready` on its LED 0, so load that one to tell them
//     apart.
//   * **LED 1 lit, LED 2 dark** — enumerated, and the processor has not
//     printed. `configured` is what lets the processor go, and it prints
//     a `\` about four milliseconds later — 240 000 clocks, measured by
//     `the_processor_runs_at_one_cycle_in_fifty_nine` — so this means the
//     6502 is not running: the ROM, the reset vector or the bus, in that
//     order.
//   * **LED 2 lit, LED 4 dark** — the monitor is running and nothing you
//     typed reached it. That is the host's end of the pipe.
//   * **LED 0 dark with LED 1 lit** — the port is open and the ACIA is
//     not using the host's rate, because the rate the host asked for is
//     one `uart_baud_div` refused. It cannot happen at any rate a
//     terminal program offers; see below.
//
// Ball C11, which is LED 5, which is the baud rate made real
//   The console is a USB pipe and a USB pipe has no bit rate, so a
//   design could report any divisor at all and nothing would notice. So
//   this one **also transmits everything the processor prints as real
//   8N1**, out of the die on ball C11, at the rate the ACIA is
//   programmed to — which is the rate the host set, through
//   `monitor_acia`'s CONTROL register and `uart_baud_div`. A bit period
//   on C11 is a number an instrument reads off a pin, and it is the only
//   thing in this design that a wrong divisor cannot hide from.
//
//   The mirror is a **diagnostic and not a channel**: one byte of
//   holding register, overwritten if the line is still busy. At 300 baud
//   a character takes 33 ms and the monitor prints faster than that, so
//   most of what it prints is dropped — and that is the right trade,
//   because the alternative is a console whose speed is set by an LED.
//   What survives is a stream of correctly framed characters at the
//   right rate, which is all the pin is for.
//
// Pins: board/cynthion.rcf.
module monitor_cynthion #(
    // How many clocks the core is held in reset after configuration.
    parameter integer POR = 16,
    parameter [6:0]   TURNAROUND = 7'd9,
    // Clocks per 6502 bus cycle, and bytes of RAM.
    parameter integer CPU_DIV = 59,
    parameter integer RAM_BYTES = 4096,
    // What the mirror falls back to when a rate cannot be divided to:
    // 521 is 115200 baud at 60 MHz, to 0.032%.
    parameter integer MIRROR_DIV = 521
) (
    input  wire clk,             // A8, the 60.000 MHz oscillator

    // The auxiliary transceiver, all on the die's right edge but D16.
    inout  wire [7:0] ulpi_data, // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire ulpi_dir,        // E16
    input  wire ulpi_nxt,        // F15
    output wire ulpi_stp,        // E15
    output wire ulpi_rst_n,      // J13, active low at the ball
    output wire ulpi_clk,        // D16, the clock the board says we owe it

    output wire led0_n,          // the divisor came from the host's rate
    output wire led1_n,          // THE HOST CONFIGURED IT
    output wire led2_n,          // THE PROCESSOR PRINTED SOMETHING
    output wire led3_n,          // heartbeat
    output wire led4_n,          // A KEY REACHED THE ACIA
    output wire led5_n           // the mirror's transmit line
);
    // -----------------------------------------------------------------
    // Power-on reset. A shift register and not a tie-high: an ECP5
    // releases its flip-flops into `REGSET` and a simulator has no
    // `REGSET`, so tying `rst_n` high would leave every core register
    // `x` for ever in simulation.
    // -----------------------------------------------------------------
    reg [POR-1:0] por = {POR{1'b0}};
    always @(posedge clk) begin
        por <= {por[POR-2:0], 1'b1};
    end
    wire reset_done = por[POR-1];

    // The board's clock, back out to the transceiver.
    assign ulpi_clk = clk;

    // The ULPI bus. No register in the turnaround path, so the pads
    // release in the same cycle the link does.
    wire [7:0] data_o;
    wire       data_oe;
    assign ulpi_data = data_oe ? data_o : 8'bz;

    // -----------------------------------------------------------------
    // The USB serial port, and the computer behind it
    //
    // One module, because that is the piece `tests/ip_library.rs` drives
    // through its transceiver model — the whole machine, through
    // `usb_device_ulpi`'s link layer, through a model of the part that
    // is on this board including its late LineState. What is left here
    // is the three-state buffers, the reset, the LEDs and the waveform.
    // -----------------------------------------------------------------
    wire        configured;
    wire [31:0] acia_rate;
    wire [7:0]  print_data;
    wire        print_valid;
    wire        out_taken;

    // DCD and DSR asserted, everything else clear: this port is always
    // connected and never breaks a frame (PSTN 1.2 6.5.4 Table 31).
    wire [6:0] serial_state = 7'b000_0011;

    monitor_ulpi #(
        .TURNAROUND (TURNAROUND),
        .CPU_DIV    (CPU_DIV),
        .RAM_BYTES  (RAM_BYTES)
    ) u_top (
        .clk60        (clk),
        .rst_n        (reset_done),
        .ulpi_data_i  (ulpi_data),
        .ulpi_data_o  (data_o),
        .ulpi_data_oe (data_oe),
        .ulpi_dir     (ulpi_dir),
        .ulpi_nxt     (ulpi_nxt),
        .ulpi_stp     (ulpi_stp),
        .ulpi_rst_n   (ulpi_rst_n),
        .address      (),
        .configured   (configured),
        .usb_reset    (),
        // Left open: `configured` on LED 1 cannot be true without it.
        .phy_ready    (),
        .serial_state (serial_state),
        .acia_rate    (acia_rate),
        .acia_control (),
        .print_data   (print_data),
        .print_valid  (print_valid),
        .key_taken    (out_taken)
    );

    // -----------------------------------------------------------------
    // The mirror on ball C11
    // -----------------------------------------------------------------
    wire [15:0] mirror_div;
    wire        baud_ok;

    uart_baud_div #(
        .CLK_HZ    (32'd60_000_000),
        .DIV_RESET (MIRROR_DIV[15:0])
    ) u_baud (
        .clk   (clk),
        .rst_n (reset_done),
        .rate  (acia_rate),
        .div   (mirror_div),
        .ok    (baud_ok),
        .busy  ()
    );

    // One byte of holding register, overwritten rather than queued. The
    // throttle is on `tx_valid` and not only on a ready signal, which is
    // the arrangement `testdata/fpga/cynthion/usb_cdc_uart.v`'s header
    // records as having been got wrong once on real hardware: `uart_tx`
    // takes a byte on its own `!busy && tx_valid`, so a byte left valid
    // after it was taken is sent twice.
    reg [7:0] mirror_q;
    reg       mirror_full;
    wire      mirror_ready;
    wire      mirror_tx;

    always @(posedge clk or negedge reset_done) begin
        if (!reset_done) begin
            mirror_q    <= 8'd0;
            mirror_full <= 1'b0;
        end else if (print_valid) begin
            mirror_q    <= print_data;
            mirror_full <= 1'b1;
        end else if (mirror_full & mirror_ready) begin
            mirror_full <= 1'b0;
        end
    end

    uart_tx #(
        .CLK_DIV (MIRROR_DIV)
    ) u_mirror (
        .clk      (clk),
        .rst_n    (reset_done),
        .div      (mirror_div),
        .tx_data  (mirror_q),
        .tx_valid (mirror_full),
        .tx_ready (mirror_ready),
        .tx       (mirror_tx)
    );

    // -----------------------------------------------------------------
    // The LEDs
    // -----------------------------------------------------------------
    // A heartbeat, written as the reduction form of `+1` rather than as
    // `count <= count + 1`, because an ECP5 slice's two flip-flops share
    // one clock-enable wire and the `if (event)` form infers one.
    reg [25:0] count = 26'd0;
    wire [25:0] toggle;
    assign toggle[0] = 1'b1;
    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : g_toggle
            assign toggle[i] = &count[i-1:0];
        end
    endgenerate
    always @(posedge clk) begin
        count <= count ^ toggle;
    end

    reg saw_configured = 1'b0;
    reg saw_print      = 1'b0;
    reg saw_key        = 1'b0;
    always @(posedge clk) begin
        saw_configured <= saw_configured | configured;
        saw_print      <= saw_print | print_valid;
        saw_key        <= saw_key | out_taken;
    end

    // Active low: a pin driven low lights one.
    assign led0_n = ~baud_ok;
    assign led1_n = ~saw_configured;
    assign led2_n = ~saw_print;
    assign led3_n = ~count[25];
    assign led4_n = ~saw_key;
    // The transmit line itself, so the LED is dark while the line idles
    // high and ball C11 carries the characters out of the part.
    assign led5_n = mirror_tx;
endmodule
