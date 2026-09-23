// apple2 — the machine: a 6502, 48 KiB of RAM, a monitor ROM, the soft
// switches and the video scanner.
//
// Everything above this is a raster and everything below it is the IP
// library. `apple2_top` wires this to `dvi_tx` and a PLL and is the thing
// a board loads; `tb/apple2_tb.v` wires it to a bare `video_timing` and
// runs it at one pixel per clock, which is the same machine at a fifth of
// the cycles. The split is deliberate: everything in here that moves with
// the picture moves on `pix_en`, so both give the same frames.
//
// Memory map. It is the Apple II's, because the point of the example is
// to build that machine rather than a machine like it:
//
//   $0000-$BFFF  RAM, 48 KiB. Zero page, the stack in page one, the
//                input buffer at $0200, and the two text pages at $0400
//                and $0800 — which the video scanner reads through a
//                second port while the processor uses the first.
//   $C000-$C0FF  the soft switches. Reading or writing one *is* the
//                operation; there is nothing to store:
//                  $C00x  KBD      read: bit 7 is the strobe, bits 6..0
//                                  the key. Only the top nibble of the
//                                  low byte is decoded, as on the real
//                                  machine, so $C000..$C00F all answer.
//                  $C01x  KBDSTRB  read or write clears the strobe.
//                  $C03x  SPKR     toggles the speaker output, which is
//                                  the whole of an Apple II's sound.
//                  $C0A0  serial card in slot 2: write to transmit.
//                  $C0A1  its status: bit 0 is "the transmitter will
//                         take a byte now".
//   $C100-$F7FF  nothing. Reads give zero. On a real machine this is the
//                slot ROM space and the language card; here it is a hole,
//                and saying so is cheaper than pretending.
//
//   $C050-$C057, the graphics soft switches, are **not decoded**: this
//   machine has no lo-res and no hi-res, and a switch that answers but
//   does nothing would be worse than one that does not answer. README.md
//   says why the line is there.
//   $F800-$FFFF  the monitor ROM, 2 KiB, with the three vectors in its
//                last six bytes. It has to be here and it has to be ROM:
//                the 6502 reads $FFFC before anything could have written
//                anywhere.
//
// The keyboard is the serial port. $C000 is fed by `uart`'s receiver
// rather than by a key matrix, so a terminal on the host is the machine's
// keyboard: type into `picocom` and the monitor sees keystrokes. That is
// one wire instead of a PS/2 decoder and a scancode table, it needs no
// second clock domain, and it makes the keyboard something a testbench
// can drive — see README.md, which argues the choice at more length. The
// transmitter is separate, at $C0A0, because the monitor echoes what it
// prints to the terminal as well as to the screen.
//
// Timing. The processor gets one bus cycle every CPU_PIX_DIV pixels. At
// the default of 14 that is one cycle per character cell fetched, which
// is the ratio a real Apple II runs at — its video reads one byte per
// processor cycle — and at this raster's 25.175 MHz pixel clock it puts
// the 6502 at 1.798 MHz against the original's 1.023. `ready` is high for
// exactly one clock per bus cycle, so the core spends the cycle counts
// its tables print, just spread out.
//
// There is no bus contention to arbitrate: the RAM has a port for the
// processor and a port for the video, so neither ever waits for the
// other. A real Apple II interleaved them on one bus, which is where its
// φ0 / φ1 split comes from; a block RAM with two ports is the same
// machine with the arbitration removed.
module apple2 #(
    // The monitor, as sw/monitor.hex, and the character generator, as
    // sw/font.hex. Both paths are relative to wherever the tool that
    // reads them runs; the commands in README.md run from this example's
    // directory.
    parameter ROM_FILE    = "sw/monitor.hex",
    parameter FONT_FILE   = "sw/font.hex",
    // Frames per state of the flashing attribute, as a power of two; see
    // rtl/apple2_video.v.
    parameter FLASH_SHIFT = 3,
    parameter ROM_BYTES   = 2048,
    parameter RAM_BYTES   = 49152,
    // Clocks per serial bit. 126 MHz / 115200 is 1094; the testbench
    // turns it right down so that typing costs a few hundred pixels
    // rather than a few hundred thousand.
    parameter CLK_DIV     = 1094,
    // Pixels per processor cycle. See above.
    parameter CPU_PIX_DIV = 14
) (
    input  wire        clk,
    input  wire        rst_n,

    // The raster, from dvi_tx (or from a bare video_timing in the
    // testbench).
    input  wire        pix_en,
    input  wire        de,
    input  wire [11:0] x,
    input  wire [11:0] y,
    output wire [7:0]  r,
    output wire [7:0]  g,
    output wire [7:0]  b,

    // The terminal that is the keyboard.
    output wire        uart_tx,
    input  wire        uart_rx,
    // $C030 toggles this. One pin, one square wave, as on the original.
    output reg         speaker
);
    localparam ROM_BITS = $clog2(ROM_BYTES);

    // -----------------------------------------------------------------
    // The core
    // -----------------------------------------------------------------
    wire [15:0] cpu_addr;
    wire [7:0]  cpu_dout;
    reg  [7:0]  cpu_din;
    wire        cpu_we;

    // One bus cycle every CPU_PIX_DIV pixels: `ready` is high for the
    // one clock that both advances a pixel and ends the cycle.
    reg [4:0] phi;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) phi <= 5'd0;
        else if (pix_en) phi <= (phi == CPU_PIX_DIV - 1) ? 5'd0 : phi + 5'd1;
    end
    wire cpu_ready = pix_en & (phi == CPU_PIX_DIV - 1);
    // The one pixel of the bus cycle in which the memories are read: the
    // byte lands one edge later, which is the pixel `cpu_ready` is in, so
    // it is there when the access completes. The address has been stable
    // since the cycle began, which is what makes one read enough.
    wire cpu_fetch = pix_en & (phi == CPU_PIX_DIV - 2);

    mos6502 #(
        // Kept on: this is a real 6502 and a monitor that prints
        // hexadecimal is one CLD away from wanting it. The Apple II's own
        // software used decimal mode, so a machine that claims to be
        // compatible cannot leave it out.
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
        // Nothing raises one. A real Apple II has no interrupt source on
        // the motherboard either: IRQ and NMI come from the slots.
        .irq        (1'b0),
        .nmi        (1'b0),
        .dbg_pc     (),
        .dbg_retire (),
        .dbg_trap   ()
    );

    // The edge that completes an access, and the one that writes.
    wire bus_write = cpu_we & cpu_ready;

    // -----------------------------------------------------------------
    // Address decode
    // -----------------------------------------------------------------
    // $C000 is 1100_0000_0000_0000, so everything below it is everything
    // that is not both of the top two bits.
    wire sel_ram = ~(cpu_addr[15] & cpu_addr[14]);
    wire sel_io  = (cpu_addr[15:8] == 8'hC0);
    wire sel_rom = (cpu_addr[15:11] == 5'b11111);

    // -----------------------------------------------------------------
    // RAM: one array, two read ports. The processor's port writes; the
    // video's only reads. Held to zero outside the RAM so the array is
    // never addressed past its end.
    // -----------------------------------------------------------------
    wire [15:0] ram_addr = cpu_addr & {16{sel_ram}};
    wire        ram_we   = bus_write & sel_ram;

    reg [7:0] ram [0:RAM_BYTES-1];
    reg [7:0] ram_q;
    always @(posedge clk) begin
        if (ram_we) ram[ram_addr] <= cpu_dout;
        if (cpu_fetch) ram_q <= ram[ram_addr];
    end

    wire [15:0] vaddr;
    wire        vfetch;
    reg  [7:0]  vdata;
    always @(posedge clk) begin
        if (pix_en & vfetch) vdata <= ram[vaddr];
    end

    // -----------------------------------------------------------------
    // ROM: the monitor, loaded once. Element 0 is $F800.
    // -----------------------------------------------------------------
    reg [7:0] rom [0:ROM_BYTES-1];
    initial $readmemh(ROM_FILE, rom);

    reg [7:0] rom_q;
    always @(posedge clk) begin
        if (cpu_fetch) rom_q <= rom[cpu_addr[ROM_BITS-1:0]];
    end

    // -----------------------------------------------------------------
    // The soft switches
    // -----------------------------------------------------------------
    // Touching one is the operation, so the decode is of the access and
    // not of a write. A read and a write do the same thing, which is why
    // 6502 code reaches them with `bit` and `sta` interchangeably.
    wire       io_touch = sel_io & cpu_ready;
    wire [3:0] io_reg   = cpu_addr[7:4];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n)                          speaker <= 1'b0;
        else if (io_touch & (io_reg == 4'h3)) speaker <= ~speaker;
    end

    // The keyboard, which is the serial receiver. A new byte always wins
    // over a clear, so a keystroke that lands in the same cycle the
    // monitor clears the strobe is not lost.
    wire [7:0] rx_data;
    wire       rx_valid;

    reg [6:0] kbd_data;
    reg       kbd_strobe;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            kbd_data   <= 7'd0;
            kbd_strobe <= 1'b0;
        end else if (rx_valid) begin
            kbd_data   <= rx_data[6:0];
            kbd_strobe <= 1'b1;
        end else if (io_touch & (io_reg == 4'h1)) begin
            kbd_strobe <= 1'b0;
        end
    end

    // The serial card in slot 2: one byte held until the transmitter
    // takes it.
    reg  [7:0] tx_byte;
    reg        tx_pending;
    wire       tx_ready;
    wire       tx_write = bus_write & sel_io & (io_reg == 4'hA) & ~cpu_addr[0];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            tx_byte    <= 8'd0;
            tx_pending <= 1'b0;
        end else if (tx_write) begin
            tx_byte    <= cpu_dout;
            tx_pending <= 1'b1;
        end else if (tx_pending & tx_ready) begin
            tx_pending <= 1'b0;
        end
    end

    // Ready only when nothing is waiting, so a poll straight after a
    // write cannot see a stale 1.
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
        .rx       (uart_rx),
        .rx_data  (rx_data),
        .rx_valid (rx_valid),
        .rx_error ()
    );

    reg [7:0] io_q;
    always @* begin
        case (io_reg)
            // $C00x and $C01x both read the key and its strobe; $C01x
            // also clears the strobe, above.
            4'h0, 4'h1: io_q = {kbd_strobe, kbd_data};
            4'hA:       io_q = cpu_addr[0] ? {7'd0, tx_free} : 8'd0;
            default:    io_q = 8'd0;
        endcase
    end

    // -----------------------------------------------------------------
    // The video
    // -----------------------------------------------------------------
    apple2_video #(
        .FONT_FILE   (FONT_FILE),
        .FLASH_SHIFT (FLASH_SHIFT)
    ) u_video (
        .clk        (clk),
        .rst_n      (rst_n),
        .pix_en     (pix_en),
        .de         (de),
        .x          (x),
        .y          (y),
        .vaddr      (vaddr),
        .vfetch     (vfetch),
        .vdata      (vdata),
        .r          (r),
        .g          (g),
        .b          (b)
    );

    // -----------------------------------------------------------------
    // Read data. The core holds its address for the whole cycle, so the
    // selects are still valid when the data is taken.
    // -----------------------------------------------------------------
    always @* begin
        if (sel_rom)      cpu_din = rom_q;
        else if (sel_io)  cpu_din = io_q;
        else if (sel_ram) cpu_din = ram_q;
        else              cpu_din = 8'd0;
    end
endmodule
