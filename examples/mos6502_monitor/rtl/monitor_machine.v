// monitor_machine — a 6502, some RAM, a 512-byte ROM and an ACIA.
//
// What it does
//   The whole computer, with a byte stream where its serial line would
//   be. `monitor_cynthion.v` puts `ip/usb/usb_cdc_acm` on the end of that
//   stream and the board's transceiver on the end of *that*; a testbench
//   drives it directly, which is what makes the monitor testable as
//   software before any of the USB stack is in the way.
//
// The memory map, and which parts of it are the board's
//   It is a published 6502 breadboard computer's, decoded by one quad
//   NAND gate, written out as the chip-select equations that gate
//   actually computes:
//
//       ROM  /CE = A15                    -> $8000-$FFFF
//       RAM  /CE = A15, /OE = A14         -> $0000-$3FFF
//       VIA  CS1 = A13, /CS2 = !A15 & A14 -> $6000-$7FFF
//       ACIA CS0 = !A13, /CS1 = !A15 & A14-> $4000-$5FFF
//
//   Three of those four are here, and each differs from the board in a
//   way this part forces:
//
//   * **The RAM window is 16 KiB and the array is 4 KiB**, so it repeats
//     four times over. That is not a hole and not a simplification: it is
//     what a smaller part in that socket does, because the decoder does
//     not look at the address bits the part does not have. `$0300` and
//     `$1300` are therefore the same byte. The reason it is 4 KiB and
//     not 16 is measured and is in README.md.
//   * **The ROM window is 32 KiB and only its top two pages are built.**
//     $FE00-$FFFF answers and $8000-$FDFF reads zero. A ROM on this part
//     is lookup tables — there is no block RAM in this flow — so a 32 KiB
//     one is not affordable and 512 bytes is — 505 `LUT4`, measured, one
//     a byte. The monitor is 266 bytes of code and six of vectors where
//     the interface it reproduces fits 256 in all; README.md reports
//     what the second page cost and why the first was not enough.
//   * **There is no timer at $6000.** The board has a 65C22 there and
//     nothing in this machine touches it, so the window reads zero.
//
// The processor's speed, and why the memories are read without a clock
//   One bus cycle every CPU_DIV clocks: `ready` is high for one clock in
//   CPU_DIV and the core holds the access through the rest, which is
//   what `ready` is for. At 60 MHz and CPU_DIV 59 that is 1.0169 MHz,
//   which is 0.6% under the 1.023 MHz the machine this map comes from
//   would be quoted at if it ran at that speed; the board it comes from
//   runs at 1.000 MHz exactly.
//
//   The RAM and the ROM are read **asynchronously** — `ram[addr]` in a
//   continuous assignment, not `q <= ram[addr]` on an edge — and there
//   are two reasons, one about this flow and one about the part:
//
//   1. A clocked read is what makes `fpga::primitives` choose a block
//      RAM, and there is no block RAM site on this fabric: the design
//      would map onto `DP16KD` and then fail to place with *"the design
//      needs N `bram` site(s) and the part has 0"*. An asynchronous read
//      is declined by the block-RAM step and lowered to
//      `TRELLIS_DPR16X4` for the RAM and to lookup tables for the ROM,
//      which are the only two things this part has.
//   2. It is also what the machine being modelled does. A 6502 with
//      static RAM and an EPROM presents an address and reads what comes
//      back; there is no clock in that path at all.
//
//   That makes the path from `addr` through the read multiplexer to the
//   core's capture flip-flop a **multi-cycle path**: the address is
//   stable for all CPU_DIV clocks of the bus cycle and only the last one
//   matters. Static timing analysis does not know that and will report
//   it against one clock period. README.md quotes the number and says so
//   rather than leaving a reader to find it in a log.
//
// What it does not do
//   No interrupts: `irq` and `nmi` are tied low, all three vectors point
//   at the reset entry, and nothing on this machine can raise one. No timer, no
//   cassette, no video. `ip/cpu/mos6502`'s own list of what a 6502 is and is
//   not applies unchanged.
module monitor_machine #(
    // Clocks per 6502 bus cycle. 59 at 60 MHz is 1.0169 MHz.
    parameter integer CPU_DIV = 59,
    // Bytes of RAM. A power of two, mirrored up through $3FFF.
    parameter integer RAM_BYTES = 4096,
    // Clocks of no output at all before a partly filled USB packet is
    // sent anyway. See `commit` below.
    parameter integer FLUSH_CLKS = 16384
) (
    input  wire        clk,
    input  wire        rst_n,

    // From the host, through the class layer.
    input  wire [7:0]  out_data,
    input  wire        out_valid,
    output wire        out_ready,
    // And back to it.
    output wire [7:0]  in_data,
    output wire        in_valid,
    input  wire        in_ready,
    output wire        in_commit,

    // What the host asked the line to be.
    input  wire [31:0] host_rate,

    // What the ACIA is programmed to, and every byte the processor
    // prints — for a design that wants to put a real waveform on a pin.
    output wire [31:0] acia_rate,
    output wire [7:0]  acia_control,
    output wire [7:0]  print_data,
    output wire        print_valid
);
    localparam integer RAM_BITS = $clog2(RAM_BYTES);

    // -----------------------------------------------------------------
    // The core
    // -----------------------------------------------------------------
    wire [15:0] cpu_addr;
    wire [7:0]  cpu_dout;
    reg  [7:0]  cpu_din;
    wire        cpu_we;

    // One bus cycle every CPU_DIV clocks.
    reg [7:0] phi;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n)                        phi <= 8'd0;
        else if (phi == CPU_DIV[7:0] - 8'd1) phi <= 8'd0;
        else                               phi <= phi + 8'd1;
    end
    wire cpu_ready = (phi == CPU_DIV[7:0] - 8'd1);

    mos6502 #(
        // Kept on. A monitor that prints hexadecimal is one stray `SED`
        // away from wanting it, and a 6502 without it is not a 6502.
        .DECIMAL_MODE (1)
    ) u_cpu (
        .clk        (clk),
        .rst_n      (rst_n),
        .addr       (cpu_addr),
        .dout       (cpu_dout),
        .din        (cpu_din),
        .we         (cpu_we),
        .ready      (cpu_ready),
        .sync       (),
        .irq        (1'b0),
        .nmi        (1'b0),
        .dbg_pc     (),
        .dbg_retire (),
        .dbg_trap   ()
    );

    // -----------------------------------------------------------------
    // The decoder: the board's four chip-select equations.
    // -----------------------------------------------------------------
    wire sel_rom  =  cpu_addr[15];
    wire sel_ram  = ~cpu_addr[15] & ~cpu_addr[14];
    wire sel_acia = ~cpu_addr[15] &  cpu_addr[14] & ~cpu_addr[13];

    wire bus_write = cpu_we & cpu_ready;

    // -----------------------------------------------------------------
    // RAM. One clocked write, one asynchronous read, no initial
    // contents and no reset: that is the shape `fpga::primitives` turns
    // into `TRELLIS_DPR16X4`, and `ip/memory/fifo_sync` and `usb_bulk_ep` are
    // the two blocks in the library already written that way.
    //
    // `ram_style` is not a hint here. It lifts the 4096-bit ceiling on
    // building a memory out of logic, which 32 768 bits is well over.
    // -----------------------------------------------------------------
    wire [RAM_BITS-1:0] ram_addr = cpu_addr[RAM_BITS-1:0];

    (* ram_style = "distributed" *)
    reg [7:0] ram [0:RAM_BYTES-1];

    always @(posedge clk) begin
        if (bus_write & sel_ram) ram[ram_addr] <= cpu_dout;
    end
    wire [7:0] ram_dout = ram[ram_addr];

    // -----------------------------------------------------------------
    // ROM. One page, and zero everywhere else in the window.
    // -----------------------------------------------------------------
    wire [7:0] rom_page;
    monitor_rom u_rom (
        .addr (cpu_addr[8:0]),
        .data (rom_page)
    );
    wire [7:0] rom_dout = (cpu_addr[15:9] == 7'b111_1111) ? rom_page : 8'h00;

    // -----------------------------------------------------------------
    // The ACIA
    // -----------------------------------------------------------------
    wire [7:0] acia_dout;
    wire [7:0] acia_rx_data  = out_data;
    wire       acia_rx_valid = out_valid;
    wire       acia_rx_ready;
    wire [7:0] acia_tx_data;
    wire       acia_tx_valid;
    wire       acia_tx_ready;

    assign out_ready = acia_rx_ready;

    monitor_acia u_acia (
        .clk       (clk),
        .rst_n     (rst_n),
        .sel       (sel_acia),
        .rs        (cpu_addr[1:0]),
        .we        (cpu_we),
        .access    (cpu_ready),
        .din       (cpu_dout),
        .dout      (acia_dout),
        .rx_data   (acia_rx_data),
        .rx_valid  (acia_rx_valid),
        .rx_ready  (acia_rx_ready),
        .tx_data   (acia_tx_data),
        .tx_valid  (acia_tx_valid),
        .tx_ready  (acia_tx_ready),
        .host_rate (host_rate),
        .rate      (acia_rate),
        .command   (),
        .control   (acia_control)
    );

    // -----------------------------------------------------------------
    // What the processor reads. A full `if`/`else` chain, because one
    // without a final `else` is a latch.
    // -----------------------------------------------------------------
    always @(*) begin
        if (sel_acia)     cpu_din = acia_dout;
        else if (sel_rom) cpu_din = rom_dout;
        else if (sel_ram) cpu_din = ram_dout;
        else              cpu_din = 8'h00;   // the empty timer window
    end

    // -----------------------------------------------------------------
    // The IN endpoint, and when a packet goes
    //
    // `usb_bulk_ep` arms itself when MAXPKT bytes have been given, so a
    // dump fills whole packets by itself. What it cannot know is when a
    // *short* one is finished — and the echo of one keystroke is a short
    // packet of one byte, which must not wait for sixty-three more.
    //
    // So `commit` is a silence: FLUSH_CLKS clocks with nothing given.
    // At 60 MHz and 16384 clocks that is 273 us, which is longer than
    // any gap between two characters the monitor prints in a row — the
    // widest is the twenty-odd processor cycles between two bytes of a
    // dump, about 25 us — and far shorter than a person notices between
    // pressing a key and seeing it.
    //
    // The counter stops at the top rather than wrapping, so a machine
    // that prints nothing for a second does not commit an empty packet
    // over and over.
    // -----------------------------------------------------------------
    assign in_data       = acia_tx_data;
    assign in_valid      = acia_tx_valid;
    assign acia_tx_ready = in_ready;

    wire gave = acia_tx_valid & in_ready;

    // `quiet` has no reset and `pending` does: the counter is read only
    // while `pending` is set, and `gave` zeroes it on the way in. That is
    // the same division `monitor_acia` uses for its byte registers, and
    // on this part it is worth asking for — a flip-flop with a reset
    // needs its tile's set/reset wire and a distributed RAM needs the
    // same wire for its write enable, so a design with 530 of the latter
    // wants as few of the former as it can manage.
    localparam [$clog2(FLUSH_CLKS):0] LAST_QUIET = FLUSH_CLKS[$clog2(FLUSH_CLKS):0];

    reg [$clog2(FLUSH_CLKS):0] quiet;
    reg                        pending;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n)                              pending <= 1'b0;
        else if (gave)                           pending <= 1'b1;
        else if (pending && quiet == LAST_QUIET) pending <= 1'b0;
    end

    // A process of its own, with no reset in it at all, which is what
    // makes the counter cost no set/reset wire. It is read only while
    // `pending` is set and `gave` zeroes it on the way in, so there is
    // nothing for a reset to do.
    always @(posedge clk) begin
        if (gave)                                quiet <= 0;
        else if (pending && quiet != LAST_QUIET) quiet <= quiet + 1'b1;
    end
    assign in_commit = pending & (quiet == LAST_QUIET);

    // Everything the processor printed, for a pin that wants it.
    assign print_data  = acia_tx_data;
    assign print_valid = gave;
endmodule
