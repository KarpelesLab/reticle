// computer_top — a MOS 6502 computer from two library blocks.
//
// This is the only HDL in the project. Everything else comes from the
// Reticle IP library through `reticle.proj`: the processor is `mos6502`
// and the serial port is `uart`. What this file adds is what a 6502 has
// always needed around it, and nothing more:
//
//   * a ROM at the top of the address space, loaded from a hex file with
//     `$readmemh`, holding the program *and* the three vectors, so the
//     program is data rather than HDL (see sw/hello.s and sw/hello.hex);
//   * a RAM at the bottom, which is where a 6502 insists its RAM goes:
//     zero page is the part's fastest addressing mode and the stack is
//     page one, and neither is relocatable;
//   * an address decoder with a memory-mapped UART behind it;
//   * a power-on reset, so the program runs as soon as the FPGA is
//     configured, with no reset pin.
//
// Memory map. The 6502 has one bus — code, data, stack and I/O all go
// through it — so there is one decode and not two:
//
//   $0000-$03FF  RAM   RAM_BYTES bytes. Zero page ($0000-$00FF) and the
//                      stack (page one, $0100-$01FF) are inside it,
//                      because on this part they have nowhere else to be.
//   $D000-$D0FF  UART  +0  DATA    write: the byte is transmitted.
//                      +1  STATUS  read:  bit 0 is 1 when a byte written
//                                         to DATA will be taken now.
//                      Only address bit 0 is decoded inside the page, so
//                      the two registers repeat every two bytes.
//   $F800-$FFFF  ROM   ROM_BYTES bytes, read only. The last six bytes are
//                      the vectors the part fetches through:
//                        $FFFA-$FFFB  NMI
//                        $FFFC-$FFFD  RES, where execution starts
//                        $FFFE-$FFFF  IRQ and BRK
//                      They have to come from ROM: the part reads $FFFC
//                      before anything has had a chance to write anywhere.
//   anything else      reads as zero; writes are ignored.
//
// Software polls STATUS until bit 0 is set and then writes DATA. A write
// while bit 0 is clear replaces the byte that is still waiting.
//
// Timing. `mos6502` performs exactly one bus access per bus cycle and
// holds the access until it sees `ready` high at a rising edge. The ROM
// and the RAM are read on a clock edge, which is what lets them become
// block RAM, so their data is one clock late: `bus_ready` is therefore
// low on the first clock of every access and high on the second. Every
// bus cycle costs two clocks, the same two for every access, so the core
// still spends the cycle counts the 6502's tables print — a 12 MHz clock
// runs it at 6 MHz, six times an original part. `ready` stalling the core
// by whole cycles rather than changing the shape of one is the whole
// reason this works; see ip/mos6502/rtl/mos6502.v.
//
// Interrupts are not used here: `irq` and `nmi` are tied low and the
// program sets I anyway. The vectors still point at real code, because a
// map with a hole at $FFFA is a map that is wrong rather than unused.
module computer_top #(
    // Clock cycles per UART bit: 12 MHz / 115200 baud = 104. The UART
    // runs on the full clock, not on the 6502's two-clock bus cycle.
    parameter CLK_DIV   = 104,
    // The program. The path is relative to wherever the tool that reads
    // it runs; the commands in README.md run from this example's
    // directory.
    parameter ROM_FILE  = "sw/hello.hex",
    // The ROM sits at the top of the address space and the RAM at the
    // bottom; both sizes are powers of two, and the decode below is
    // derived from them rather than written out again.
    parameter ROM_BYTES = 2048,
    parameter RAM_BYTES = 1024
) (
    input  wire clk,
    // The serial port, 8N1.
    output wire uart_tx,
    input  wire uart_rx
);
    localparam ROM_BITS = $clog2(ROM_BYTES);
    localparam RAM_BITS = $clog2(RAM_BYTES);
    // The first address the ROM answers to: $10000 - ROM_BYTES.
    localparam ROM_BASE = 32'h0001_0000 - ROM_BYTES;

    // -----------------------------------------------------------------
    // Reset: the system resets itself. `rst_n` is held low for the first
    // eight clocks and then released for good. An iCE40 starts every
    // flip-flop at zero when it is configured, which is what the initial
    // value says, so this is a power-on reset on a board and the reset at
    // the start of a simulation. Releasing it starts the core's own
    // seven-cycle RES sequence, which ends by jumping through $FFFC.
    // -----------------------------------------------------------------
    reg [3:0] por_count = 4'd0;
    always @(posedge clk) begin
        if (!por_count[3]) por_count <= por_count + 4'd1;
    end
    wire rst_n = por_count[3];

    // -----------------------------------------------------------------
    // The core
    // -----------------------------------------------------------------
    wire [15:0] cpu_addr;
    wire [7:0]  cpu_dout;
    reg  [7:0]  cpu_din;
    wire        cpu_we;
    reg         bus_ready;

    mos6502 #(
        // Packed binary-coded decimal costs two adders and a pair of
        // comparators, and nothing here counts in decimal, so it is not
        // built. D is still a flag; SED, CLD, PHP and PLP still see it.
        .DECIMAL_MODE (0)
    ) u_cpu (
        .clk        (clk),
        .rst_n      (rst_n),
        .addr       (cpu_addr),
        .dout       (cpu_dout),
        .din        (cpu_din),
        .we         (cpu_we),
        .ready      (bus_ready),
        // Nothing here follows the opcode fetches or the retirement
        // trace; a testbench would.
        .sync       (),
        // No interrupt sources in this system.
        .irq        (1'b0),
        .nmi        (1'b0),
        .dbg_pc     (),
        .dbg_retire (),
        .dbg_trap   ()
    );

    // -----------------------------------------------------------------
    // The bus: one wait state on every access, because the memories are
    // read on a clock edge. `bus_ready` is low for the first clock of an
    // access and high for the second, which is the edge that transfers.
    // -----------------------------------------------------------------
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) bus_ready <= 1'b0;
        else        bus_ready <= ~bus_ready;
    end

    // The edge that completes an access, and the one that writes.
    wire bus_write = cpu_we & bus_ready;

    // -----------------------------------------------------------------
    // Address decode
    // -----------------------------------------------------------------
    wire sel_ram  = cpu_addr < RAM_BYTES;
    wire sel_uart = cpu_addr[15:8] == 8'hD0;
    wire sel_rom  = cpu_addr >= ROM_BASE;

    // Word offset 0 within the UART page is DATA, 1 is STATUS.
    wire uart_data   = sel_uart & ~cpu_addr[0];
    wire uart_status = sel_uart &  cpu_addr[0];

    // -----------------------------------------------------------------
    // ROM: loaded once from ROM_FILE. One read port, because the 6502
    // makes one access per cycle whatever it is fetching, so the ROM is a
    // single copy in block RAM. Element 0 is address ROM_BASE, which is
    // the numbering sw/hello.hex is written in.
    // -----------------------------------------------------------------
    reg [7:0] rom [0:ROM_BYTES-1];
    initial $readmemh(ROM_FILE, rom);

    reg [7:0] rom_q;
    always @(posedge clk) begin
        rom_q <= rom[cpu_addr[ROM_BITS-1:0]];
    end

    // -----------------------------------------------------------------
    // RAM: one byte-wide array, since the 6502 writes one byte at a time
    // and never less.
    // -----------------------------------------------------------------
    reg [7:0] ram [0:RAM_BYTES-1];
    wire ram_write = bus_write & sel_ram;

    reg [7:0] ram_q;
    always @(posedge clk) begin
        if (ram_write) ram[cpu_addr[RAM_BITS-1:0]] <= cpu_dout;
        ram_q <= ram[cpu_addr[RAM_BITS-1:0]];
    end

    // -----------------------------------------------------------------
    // UART: DATA holds one byte until the transmitter takes it.
    // -----------------------------------------------------------------
    reg  [7:0] tx_byte;
    reg        tx_pending;
    wire       tx_ready;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            tx_byte    <= 8'd0;
            tx_pending <= 1'b0;
        end else if (bus_write & uart_data) begin
            tx_byte    <= cpu_dout;
            tx_pending <= 1'b1;
        end else if (tx_pending & tx_ready) begin
            // The transmitter took the byte on this edge.
            tx_pending <= 1'b0;
        end
    end

    // Ready for a new byte only when nothing is waiting and the shifter
    // is idle, so a poll straight after a write cannot see a stale 1.
    wire tx_free = tx_ready & ~tx_pending;

    uart #(
        .CLK_DIV  (CLK_DIV)
    ) u_uart (
        .clk      (clk),
        .rst_n    (rst_n),
        .tx_data  (tx_byte),
        .tx_valid (tx_pending),
        .tx_ready (tx_ready),
        .tx       (uart_tx),
        // The receiver is wired to the pin but nothing reads it yet.
        .rx       (uart_rx),
        .rx_data  (),
        .rx_valid (),
        .rx_error ()
    );

    // -----------------------------------------------------------------
    // Read data: the region is still selected, because the core holds its
    // address until the access completes.
    // -----------------------------------------------------------------
    always @* begin
        if (sel_rom)           cpu_din = rom_q;
        else if (sel_ram)      cpu_din = ram_q;
        else if (uart_status)  cpu_din = {7'd0, tx_free};
        else                   cpu_din = 8'd0;
    end
endmodule
