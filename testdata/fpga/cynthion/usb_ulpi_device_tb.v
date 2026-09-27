// What the **top level** of `usb_ulpi_device.v` adds to the core, checked.
//
// The USB device itself — the packets, the CRCs, the enumeration — is
// checked in `tests/ip_library.rs` against a transceiver model and a host
// model written in Rust, and this does not repeat any of it. What is left
// over, and is what a top level is, is four things:
//
//   1. the eight-bit **turnaround**: `ulpi_data` is driven when and only
//      when the link says it owns the bus, and released in the same instant
//      `dir` rises, with no clock edge in between;
//   2. the input side of the same eight pads reaching the core, which is
//      what makes the register readback in its start-up sequence possible;
//   3. the **power-on reset**, which is the whole reason the core can be
//      simulated in this top level at all — with `rst_n` tied high, every
//      register of the core would stay unknown here forever, however well
//      the part's own `REGSET` behaves;
//   4. the interface clock leaving the part on its own pin.
//
// So this is the smallest transceiver that gets the core through its
// start-up: it acknowledges what it is sent with `nxt`, and for the register
// reads the core does it takes the bus and hands back what was written. Its
// register file is 64 bytes and holds any address, which is why the board's
// own vendor register at `39h` needed nothing added here beyond the
// assertion that it was written. If
// the start-up conversation is right, `led0_n` — `phy_ready` — goes low, and
// that is the verdict.
//
// One thing it is deliberately not the smallest possible version of: **its
// pair starts at SE0.** The Debug register's LineState answers `00h` twice
// before it answers `01h`, because that is what a real transceiver does — the
// 1.5 kOhm pull-up the core has just connected has to charge the pair, and on
// a Microchip part on a Cynthion it reads SE0 for milliseconds first
// (`docs/fpga-trellis.md`). A model that answers J straight away cannot tell
// a Link that reads LineState once from one that reads it until it settles,
// and the difference is a device that works from one that holds `usb_reset`
// for ever. `tests/ip_library.rs` has the same correction in its own model.
//
// `POR` is 4 rather than the board's 16; nothing else is changed.
//
// Run with:
//   reticle sim --top usb_ulpi_device_tb usb_ulpi_device_tb.v \
//       usb_ulpi_device.v ../../../ip/usb_device_ulpi/rtl/usb_ulpi_link.v \
//       ../../../ip/usb_device_ulpi/rtl/usb_device_ulpi.v \
//       ../../../ip/usb_device_fs/rtl/usb_ctrl_ep.v
`timescale 1ps/1ps
module usb_ulpi_device_tb;

    reg clk = 1'b0;
    always #8333 clk = ~clk;   // 60 MHz, half a period 8333 ps

    // The transceiver's side of the bus.
    reg        dir     = 1'b0;
    reg        nxt     = 1'b0;
    reg  [7:0] phy_out = 8'h00;
    reg        phy_oe  = 1'b0;

    wire [7:0] data;
    wire       stp, rst_n_out, clk_out;
    wire       led0_n, led1_n, led2_n, led3_n, led4_n, led5_n;

    assign data = phy_oe ? phy_out : 8'bz;

    usb_ulpi_device #(.POR(4)) dut (
        .clk        (clk),
        .ulpi_data  (data),
        .ulpi_dir   (dir),
        .ulpi_nxt   (nxt),
        .ulpi_stp   (stp),
        .ulpi_rst_n (rst_n_out),
        .ulpi_clk   (clk_out),
        .led0_n     (led0_n),
        .led1_n     (led1_n),
        .led2_n     (led2_n),
        .led3_n     (led3_n),
        .led4_n     (led4_n),
        .led5_n     (led5_n)
    );

    task check;
        input wrong;
        input [72*8-1:0] what;
        begin
            if (wrong === 1'b1) begin
                $display("FAIL: %0s", what);
                $finish;
            end
        end
    endtask

    // ---------------------------------------------------------------
    // The smallest transceiver that gets the core through start-up
    // ---------------------------------------------------------------
    //
    // A register write is `10aaaaaa`, the byte, then `stp`; a register read
    // is `11aaaaaa`, then the transceiver takes the bus for one turnaround
    // cycle and drives the byte. `nxt` acknowledges each byte of a write and
    // the command of a read. Nothing here decodes a transmit command,
    // because the core sends none until a host does something.
    reg [7:0] regs [0:63];
    reg [5:0] address = 6'd0;
    reg [2:0] phase = 3'd0;
    integer   writes = 0;
    integer   reads = 0;
    // Debug register reads so far, so the first two can answer SE0.
    integer   line_reads = 0;

    // The phases, and the cycle each one is. `nxt` and `dir` are registered
    // here, as a transceiver's are, so every acknowledgement arrives the
    // cycle after the one it is about — which is why a write takes a wait
    // state: the Link holds its command until it has seen `nxt`, so the
    // cycle after the command still has the command on the bus, and the byte
    // comes the cycle after that.
    localparam [2:0] P_IDLE  = 3'd0;   // watching for a command
    localparam [2:0] P_ACK   = 3'd1;   // `nxt` is out, the command is still up
    localparam [2:0] P_DATA  = 3'd2;   // the byte of a write
    localparam [2:0] P_TURN  = 3'd3;   // a read: the transceiver takes the bus
    localparam [2:0] P_READ  = 3'd4;   // a read: it drives the byte

    integer r;
    initial begin
        for (r = 0; r < 64; r = r + 1) regs[r] = 8'h00;
    end

    always @(posedge clk) begin
        nxt    <= 1'b0;
        dir    <= 1'b0;
        phy_oe <= 1'b0;
        case (phase)
            P_IDLE: begin
                if (rst_n_out === 1'b1 && data[7] === 1'b1) begin
                    address <= data[5:0];
                    nxt     <= 1'b1;
                    // Bit 6 tells a read from a write: `11aaaaaa` against
                    // `10aaaaaa`.
                    phase   <= data[6] ? P_TURN : P_ACK;
                end
            end
            P_ACK:  phase <= P_DATA;
            P_DATA: begin
                regs[address] <= data;
                nxt           <= 1'b1;
                writes        <= writes + 1;
                phase         <= P_IDLE;
            end
            P_TURN: begin
                dir    <= 1'b1;
                phy_oe <= 1'b1;
                // The Debug register's LineState: SE0 while the pull-up the
                // core has just connected charges the pair, and then J, which
                // is where a full-speed bus idles. Everything else reads back
                // what was written.
                if (address == 6'h15) begin
                    phy_out    <= (line_reads < 2) ? 8'h00 : 8'h01;
                    line_reads <= line_reads + 1;
                end else begin
                    phy_out <= regs[address];
                end
                phase  <= P_READ;
            end
            P_READ: begin
                dir    <= 1'b1;
                phy_oe <= 1'b1;
                reads  <= reads + 1;
                phase  <= P_IDLE;
            end
            default: phase <= P_IDLE;
        endcase
    end

    // ---------------------------------------------------------------
    // THE TURNAROUND, watched every cycle rather than sampled
    // ---------------------------------------------------------------
    //
    // Two things must never happen, and a check that only looked at the end
    // of the run would miss both: the two drivers on the bus at once, and
    // the FPGA driving while the transceiver owns it.
    reg saw_fpga_drive = 1'b0;
    always @(posedge clk) begin
        if (dir === 1'b1 && phy_oe === 1'b1 && data !== phy_out) begin
            $display("FAIL: the FPGA drove the bus while the transceiver had it");
            $finish;
        end
        if (dir === 1'b0 && data !== 8'bzzzz_zzzz) saw_fpga_drive <= 1'b1;
    end

    integer settle;
    initial begin
        // The transceiver's reset pin is held at first and then released:
        // the core's own start-up, not this top level's, but if the power-on
        // reset did not work nothing would move at all.
        //
        // Two edges first, and not because two is a magic number: the core's
        // reset is `posedge clk or negedge rst_n`, and `rst_n` starts **at**
        // zero rather than falling to it, so the reset takes effect on the
        // first clock edge and not before. Everything is unknown until then,
        // which is exactly the state a design with `rst_n` tied high would
        // never leave.
        repeat (2) @(posedge clk);
        check(rst_n_out !== 1'b0, "the transceiver's reset was never asserted");
        // 300 clocks of reset and 300 of waiting for an idle bus, from the
        // core's RESET_CYCLES.
        for (settle = 0; settle < 800; settle = settle + 1) @(posedge clk);
        check(rst_n_out !== 1'b1, "the transceiver was never let out of reset");
        check(clk_out !== clk, "the interface clock is not the board's clock");

        // The start-up conversation: three register writes and two reads,
        // and then `phy_ready`. `tests/ip_library.rs` asserts which
        // registers and which bytes against a fuller model; what matters
        // here is that the bytes got through the pads in both directions.
        for (settle = 0; settle < 4000; settle = settle + 1) @(posedge clk);
        check(writes < 4, "fewer than four register writes reached the transceiver");
        check(reads < 3, "the core never read a register back through the pads");
        check(
            line_reads < 3,
            "the core took the first LineState answer instead of reading again"
        );
        check(
            regs[6'h04] !== 8'h45,
            "Function Control is not the full-speed peripheral value 45h"
        );
        check(regs[6'h0A] !== 8'h00, "OTG Control still has a host's pull-downs");
        // The board's own register, and the reason this design works at all:
        // 39h bit 1 is the USB3343's `SwapDP/DM`, and this board crosses DP
        // and DM between the transceiver and the connector. The top level's
        // header has the three sources. A write that did not reach the part
        // is a device a host detects as low speed.
        check(
            regs[6'h39] !== 8'h06,
            "the transceiver was never told that this board crosses DP and DM"
        );
        check(led0_n !== 1'b0, "LED 0 is dark, so `phy_ready` never came up");
        check(!saw_fpga_drive, "the FPGA never drove the bus at all");

        // Nothing has happened on the USB, so the rest of the LEDs are dark.
        check(led1_n !== 1'b1, "LED 1 is lit and no host has configured anything");
        check(led2_n !== 1'b1, "LED 2 is lit and no host has assigned an address");
        check(led4_n !== 1'b1, "LED 4 is lit and no host has reset the bus");
        check(led5_n !== 1'b0, "LED 5 is dark though the transceiver took the bus");

        $display("PASS: the transceiver was configured and read back through the pads");
        $finish;
    end
endmodule
