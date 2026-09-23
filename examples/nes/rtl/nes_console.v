// nes_console — an NES-compatible console: processor, picture unit,
// memory map and cartridge.
//
// This is the machine, without anything to do with a monitor. It takes
// a clock and a dot enable and it produces a picture on `vid_*`, one
// pixel per enabled clock. `nes_top` is what puts a framebuffer and a
// DVI transmitter behind that port; `tb/nes_console_tb.v` is what runs
// it on its own.
//
// The two library blocks it is built from are `mos6502` and `ppu2c02`.
// Everything else on this page is the console board: 2 KiB of work RAM,
// the decode that mirrors it and the eight picture registers across the
// whole address space, the sprite DMA engine at $4014, the nametable
// memory with its mirroring, and an NROM cartridge.
//
// **The processor is `mos6502` with `DECIMAL_MODE = 0`.** That is not
// an optimisation, it is the part: the 2A03 in an NES is a 6502 with
// decimal mode disabled in silicon — D is still a flag, SED and CLD and
// PHP still see it, and ADC and SBC ignore it. So the parameter that
// exists in the library block to save two adders happens to be exactly
// the difference between a 6502 and the processor in this console, and
// setting it to zero is what makes the core correct here rather than
// merely smaller.
//
// Clocking. On a real console one master oscillator is divided by four
// for the picture unit and by twelve for the processor, so the picture
// unit runs three times as fast. Here `en` is one dot and the processor
// gets every third one:
//
//     en      . . . . . . . . . . . .   one dot each
//     cpu_ph  0 1 2 0 1 2 0 1 2 0 1 2
//     ready         ^       ^       ^   one processor cycle each
//
// `mos6502` performs exactly one bus access per bus cycle and holds it
// until it sees `ready` high at a rising edge, so holding `ready` low
// for two dots out of three is all it takes to run it at a third of the
// dot rate — and holding it low for a few hundred cycles is all it
// takes to stop it for a sprite DMA. Memory is read on an `en` edge,
// which is what lets it become block RAM, and the address has been
// stable since the previous processor cycle, so the byte is there by
// the time `ready` goes high.
//
// The map. One bus, as on any 6502:
//
//   $0000-$07FF  RAM, 2 KiB. Zero page, the stack in page one, and the
//                rest. Mirrored three more times up to $1FFF, because
//                the console decodes eleven address bits and not
//                thirteen — a program really can find its stack at
//                $1100, and some do.
//   $2000-$2007  the picture unit's eight registers, mirrored every
//                eight bytes up to $3FFF for the same reason.
//   $4014        sprite DMA: writing $XX copies $XX00-$XXFF into OAM
//                and stops the processor for 513 cycles.
//   $4000-$4017  otherwise the sound and controller registers, which
//                this console does not have: writes are dropped and
//                reads give zero.
//   $8000-$FFFF  the cartridge's program ROM. This is an NROM board
//                with 16 KiB, so $8000 and $C000 are the same memory
//                and the vectors at $FFFA-$FFFF are the top of it.
//
// The cartridge also carries 8 KiB of pattern memory on the picture
// unit's own bus, and wires the two nametables either side by side or
// one above the other. MIRROR picks which: vertical mirroring puts them
// side by side, which is what a program scrolling sideways wants.
module nes_console #(
    // The program, as a hex image of the 16 KiB program ROM.
    parameter PRG_FILE  = "sw/demo.hex",
    // The pattern memory: 8 KiB, two tables of 256 tiles.
    parameter CHR_FILE  = "sw/chr.hex",
    parameter PRG_BYTES = 16384,
    // 1 is vertical mirroring (nametables side by side), 0 horizontal.
    parameter MIRROR    = 1
) (
    input  wire        clk,
    input  wire        rst_n,
    // One dot of the picture unit; the processor gets every third.
    input  wire        en,

    // The picture, one pixel per `en` while `vid_de` is high.
    output wire        vid_de,
    output wire [7:0]  vid_x,
    output wire [7:0]  vid_y,
    output wire [5:0]  vid_color,
    output wire        vid_frame,

    // For a testbench.
    output wire [8:0]  dbg_dot,
    output wire [8:0]  dbg_line,
    output wire [15:0] dbg_pc,
    output wire        dbg_retire,
    output wire        dbg_dma
);
    localparam PRG_BITS = $clog2(PRG_BYTES);

    // -----------------------------------------------------------------
    // The processor's cycle out of three dots
    // -----------------------------------------------------------------
    reg [1:0] cpu_ph;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n)   cpu_ph <= 2'd0;
        else if (en)  cpu_ph <= (cpu_ph == 2'd2) ? 2'd0 : cpu_ph + 2'd1;
    end

    wire cpu_tick  = en & (cpu_ph == 2'd2);

    // -----------------------------------------------------------------
    // The processor
    // -----------------------------------------------------------------
    wire [15:0] cpu_addr;
    wire [7:0]  cpu_dout;
    reg  [7:0]  cpu_din;
    wire        cpu_we;
    wire        ppu_nmi;

    wire dma_active;
    wire cpu_ready = cpu_tick & ~dma_active;
    wire cpu_write = cpu_we & cpu_ready;

    mos6502 #(
        // An NES processor is a 6502 with decimal mode disabled. See
        // the note at the top of this file: this is the part, not a
        // saving.
        .DECIMAL_MODE (0)
    ) u_cpu (
        .clk        (clk),
        .rst_n      (rst_n),
        .addr       (cpu_addr),
        .dout       (cpu_dout),
        .din        (cpu_din),
        .we         (cpu_we),
        .ready      (cpu_ready),
        .sync       (),
        // Nothing in an NROM console raises an IRQ: there is no sound
        // hardware to interrupt at the end of a frame counter step and
        // no mapper with a scanline counter.
        .irq        (1'b0),
        .nmi        (ppu_nmi),
        .dbg_pc     (dbg_pc),
        .dbg_retire (dbg_retire),
        .dbg_trap   ()
    );

    // -----------------------------------------------------------------
    // Sprite DMA
    //
    // Writing a page number to $4014 copies that whole page into the
    // picture unit's OAM, one byte per processor cycle, through the
    // same $2004 port a program would use — which is what the real
    // console does and why OAMADDR ends up back where it started.
    // The processor is stopped for the whole of it by `ready`.
    //
    // 513 cycles: one to line up, then 256 pairs of a read and a write.
    // The real part spends a 514th when the write to $4014 lands on an
    // odd cycle, which this one does not model.
    // -----------------------------------------------------------------
    reg        dma_run;
    reg [7:0]  dma_page;
    reg [7:0]  dma_index;
    reg [9:0]  dma_step;
    reg [7:0]  dma_byte;

    assign dma_active = dma_run;
    assign dbg_dma    = dma_run;

    wire dma_start = cpu_write & (cpu_addr == 16'h4014);
    // The even steps write what the odd steps read.
    wire dma_write = dma_run & cpu_tick & (dma_step != 10'd0) & ~dma_step[0];
    wire dma_read  = dma_run & cpu_tick & dma_step[0];

    // The address on the console's bus: the processor's, or the DMA
    // engine's while it has the machine.
    wire [15:0] bus_addr = dma_run ? {dma_page, dma_index} : cpu_addr;

    // -----------------------------------------------------------------
    // Memory
    // -----------------------------------------------------------------
    reg [7:0] ram [0:2047];
    reg [7:0] prg [0:PRG_BYTES-1];
    reg [7:0] ram_q;
    reg [7:0] prg_q;

    initial $readmemh(PRG_FILE, prg);

    wire sel_ram = (bus_addr[15:13] == 3'b000);
    wire sel_prg = bus_addr[15];
    wire ram_we  = cpu_write & (cpu_addr[15:13] == 3'b000);

    always @(posedge clk) begin
        if (en) begin
            if (ram_we) ram[bus_addr[10:0]] <= cpu_dout;
            ram_q <= ram[bus_addr[10:0]];
            prg_q <= prg[bus_addr[PRG_BITS-1:0]];
        end
    end

    // What the bus answers with, for the processor and for the DMA
    // engine alike. Anything the console does not decode reads as zero.
    reg [7:0] bus_q;
    always @* begin
        if (sel_prg)      bus_q = prg_q;
        else if (sel_ram) bus_q = ram_q;
        else              bus_q = 8'h00;
    end

    // -----------------------------------------------------------------
    // The picture unit
    // -----------------------------------------------------------------
    wire        ppu_sel = (cpu_addr[15:13] == 3'b001);
    wire [7:0]  ppu_dout;
    wire [13:0] ppu_addr;
    wire [7:0]  ppu_wdata;
    wire        ppu_we;
    reg  [7:0]  ppu_rdata;

    // The DMA engine reaches OAM through $2004, exactly as a program
    // writing one sprite at a time would.
    wire [2:0] ppu_reg_addr = dma_write ? 3'd4      : cpu_addr[2:0];
    wire [7:0] ppu_reg_din  = dma_write ? dma_byte  : cpu_dout;
    wire       ppu_reg_we   = dma_write | (ppu_sel & cpu_write);
    wire       ppu_reg_re   = ppu_sel & ~cpu_we & cpu_ready;

    ppu2c02 u_ppu (
        .clk       (clk),
        .rst_n     (rst_n),
        .en        (en),
        .reg_addr  (ppu_reg_addr),
        .reg_din   (ppu_reg_din),
        .reg_dout  (ppu_dout),
        .reg_we    (ppu_reg_we),
        .reg_re    (ppu_reg_re),
        .nmi       (ppu_nmi),
        .vram_addr (ppu_addr),
        .vram_dout (ppu_wdata),
        .vram_we   (ppu_we),
        .vram_din  (ppu_rdata),
        .vid_de    (vid_de),
        .vid_x     (vid_x),
        .vid_y     (vid_y),
        .vid_color (vid_color),
        .vid_frame (vid_frame),
        .dbg_dot   (dbg_dot),
        .dbg_line  (dbg_line)
    );

    always @* begin
        if (ppu_sel)      cpu_din = ppu_dout;
        else              cpu_din = bus_q;
    end

    // -----------------------------------------------------------------
    // The cartridge's side of the picture bus
    //
    // $0000-$1FFF is the cartridge's pattern memory, read only on an
    // NROM board. $2000-$3EFF is the console's two nametables, which
    // the cartridge wires into four: MIRROR = 1 makes address bit 10
    // the one that chooses, so the two sit side by side and a program
    // can scroll sideways through them; MIRROR = 0 makes it bit 11, so
    // they sit one above the other. Bit 12 is not decoded, which is
    // what makes $3000-$3EFF the mirror of $2000-$2EFF that it is.
    // -----------------------------------------------------------------
    reg [7:0] chr [0:8191];
    reg [7:0] nt  [0:2047];
    reg [7:0] chr_q;
    reg [7:0] nt_q;
    reg       nt_sel_q;

    initial $readmemh(CHR_FILE, chr);

    wire        nt_sel   = ppu_addr[13];
    wire [10:0] nt_index = {MIRROR ? ppu_addr[10] : ppu_addr[11], ppu_addr[9:0]};

    always @(posedge clk) begin
        if (en) begin
            if (ppu_we & nt_sel) nt[nt_index] <= ppu_wdata;
            nt_q     <= nt[nt_index];
            chr_q    <= chr[ppu_addr[12:0]];
            nt_sel_q <= nt_sel;
        end
    end

    always @* begin
        if (nt_sel_q) ppu_rdata = nt_q;
        else          ppu_rdata = chr_q;
    end

    // -----------------------------------------------------------------
    // The DMA engine's own state
    // -----------------------------------------------------------------
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            dma_run   <= 1'b0;
            dma_page  <= 8'd0;
            dma_index <= 8'd0;
            dma_step  <= 10'd0;
            dma_byte  <= 8'd0;
        end else if (en) begin
            if (dma_read)  dma_byte  <= bus_q;
            if (dma_write) dma_index <= dma_index + 8'd1;
            if (dma_run && cpu_tick) begin
                if (dma_step == 10'd512) dma_run <= 1'b0;
                else                     dma_step <= dma_step + 10'd1;
            end
            if (dma_start) begin
                dma_run   <= 1'b1;
                dma_page  <= cpu_dout;
                dma_index <= 8'd0;
                dma_step  <= 10'd0;
            end
        end
    end
endmodule
