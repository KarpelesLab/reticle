// soc_top — a RISC-V system on chip from two library blocks.
//
// This is the only HDL in the project. Everything else comes from the
// Reticle IP library through `reticle.proj`: the processor is `rv32i` and
// the serial port is `uart`. What this file adds is what every small
// system needs around a core, and nothing more:
//
//   * an instruction ROM, loaded from a hex file with `$readmemh`, so the
//     program is data rather than HDL (see sw/hello.s and sw/hello.hex);
//   * a data RAM, byte-writable, for the stack;
//   * an address decoder with a memory-mapped UART behind it;
//   * a power-on reset, so the program runs as soon as the FPGA is
//     configured, with no reset pin.
//
// Memory map (byte addresses; the decode uses bits 31:16 in full, and
// inside a region the low address bits wrap):
//
//   0x0000_0000  ROM   ROM_WORDS x 32 bits. Instruction fetch reads it,
//                      and so do loads, which is how the program reads
//                      its string. Writes are ignored. Only the data port
//                      is decoded: every fetch goes to the ROM.
//   0x0001_0000  RAM   RAM_WORDS x 32 bits, read and written by loads and
//                      stores of any width.
//   0x0002_0000  UART  +0  TXDATA  write: the low byte is transmitted.
//                      +4  STATUS  read:  bit 0 is 1 when a byte written
//                                         to TXDATA will be taken now.
//   anything else      reads as zero; writes are ignored.
//
// Software polls STATUS until bit 0 is set and then writes TXDATA. A
// write while bit 0 is clear replaces the byte that is still waiting.
//
// Timing. Both memory ports use the core's request/ready handshake with
// one wait state: the ROM and the RAM are read on a clock edge (which is
// what lets them become block RAM), and `*_ready` rises in the next cycle
// with the data. So an instruction takes three clocks, or five when it
// touches data memory.
module soc_top #(
    // Clock cycles per UART bit: 12 MHz / 115200 baud = 104.
    parameter CLK_DIV   = 104,
    // The program. The path is relative to wherever the tool that reads
    // it runs; the commands in README.md run from examples/soc.
    parameter ROM_FILE  = "sw/hello.hex",
    // Memory sizes, in 32-bit words. Powers of two.
    parameter ROM_WORDS = 256,
    parameter RAM_WORDS = 256
) (
    input  wire clk,
    // The serial port, 8N1.
    output wire uart_tx,
    input  wire uart_rx
);
    localparam ROM_BITS = $clog2(ROM_WORDS);
    localparam RAM_BITS = $clog2(RAM_WORDS);

    // -----------------------------------------------------------------
    // Reset: the system resets itself. `rst_n` is held low for the first
    // eight clocks and then released for good. An iCE40 starts every
    // flip-flop at zero when it is configured, which is what the initial
    // value says, so this is a power-on reset on a board and the reset at
    // the start of a simulation. The board this targets has no reset
    // button; add an input here and AND it into `rst_n` if yours does.
    // -----------------------------------------------------------------
    reg [3:0] por_count = 4'd0;
    always @(posedge clk) begin
        if (!por_count[3]) por_count <= por_count + 4'd1;
    end
    wire rst_n = por_count[3];

    // -----------------------------------------------------------------
    // The core
    // -----------------------------------------------------------------
    wire [31:0] imem_addr;
    wire        imem_req;
    reg         imem_ready;
    wire [31:0] imem_rdata;

    wire [31:0] dmem_addr;
    wire        dmem_req;
    wire        dmem_we;
    wire [3:0]  dmem_be;
    wire [31:0] dmem_wdata;
    reg         dmem_ready;
    wire [31:0] dmem_rdata;

    rv32i #(
        .RESET_VECTOR (32'h0000_0000),
        // The register file in block RAM: it is what makes the core fit
        // comfortably (see reticle.proj).
        .REGFILE_BRAM (1)
    ) u_cpu (
        .clk          (clk),
        .rst_n        (rst_n),
        .imem_addr    (imem_addr),
        .imem_req     (imem_req),
        .imem_ready   (imem_ready),
        .imem_rdata   (imem_rdata),
        .dmem_addr    (dmem_addr),
        .dmem_req     (dmem_req),
        .dmem_we      (dmem_we),
        .dmem_be      (dmem_be),
        .dmem_wdata   (dmem_wdata),
        .dmem_ready   (dmem_ready),
        .dmem_rdata   (dmem_rdata),
        // No interrupt sources in this system.
        .irq_timer    (1'b0),
        .irq_software (1'b0),
        .irq_external (1'b0),
        .dbg_pc       (),
        .dbg_retire   (),
        .dbg_trap     ()
    );

    // -----------------------------------------------------------------
    // Address decode for the data port
    // -----------------------------------------------------------------
    wire sel_rom  = dmem_addr[31:16] == 16'h0000;
    wire sel_ram  = dmem_addr[31:16] == 16'h0001;
    wire sel_uart = dmem_addr[31:16] == 16'h0002;

    // The edge that completes a data access: the core is still asking and
    // this is the cycle the answer is on the bus.
    wire dmem_done = dmem_req & dmem_ready;
    wire dmem_write = dmem_done & dmem_we;

    // One wait state on each port, for the synchronous reads below.
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            imem_ready <= 1'b0;
            dmem_ready <= 1'b0;
        end else begin
            imem_ready <= imem_req & ~imem_ready;
            dmem_ready <= dmem_req & ~dmem_ready;
        end
    end

    // -----------------------------------------------------------------
    // ROM: loaded once from ROM_FILE, with one read port shared by both
    // buses. rv32i is multi-cycle and never asks on both ports in the
    // same cycle (it fetches, or it accesses data, never both), so one
    // port serves both and the ROM is a single copy in block RAM rather
    // than one copy per bus.
    // -----------------------------------------------------------------
    reg [31:0] rom [0:ROM_WORDS-1];
    initial $readmemh(ROM_FILE, rom);

    wire [ROM_BITS-1:0] rom_index = dmem_req ? dmem_addr[ROM_BITS+1:2]
                                             : imem_addr[ROM_BITS+1:2];
    reg  [31:0]         rom_q;
    always @(posedge clk) begin
        rom_q <= rom[rom_index];
    end
    wire [31:0] rom_rdata = rom_q;
    assign imem_rdata = rom_q;

    // -----------------------------------------------------------------
    // RAM: one array per byte lane, so a byte or half-word store writes
    // exactly its lanes and each lane stays a plain block RAM.
    // -----------------------------------------------------------------
    reg [7:0] ram0 [0:RAM_WORDS-1];
    reg [7:0] ram1 [0:RAM_WORDS-1];
    reg [7:0] ram2 [0:RAM_WORDS-1];
    reg [7:0] ram3 [0:RAM_WORDS-1];

    wire [RAM_BITS-1:0] ram_index = dmem_addr[RAM_BITS+1:2];
    wire                ram_write = dmem_write & sel_ram;

    reg [7:0] ram0_q, ram1_q, ram2_q, ram3_q;
    always @(posedge clk) begin
        if (ram_write & dmem_be[0]) ram0[ram_index] <= dmem_wdata[7:0];
        if (ram_write & dmem_be[1]) ram1[ram_index] <= dmem_wdata[15:8];
        if (ram_write & dmem_be[2]) ram2[ram_index] <= dmem_wdata[23:16];
        if (ram_write & dmem_be[3]) ram3[ram_index] <= dmem_wdata[31:24];
        ram0_q <= ram0[ram_index];
        ram1_q <= ram1[ram_index];
        ram2_q <= ram2[ram_index];
        ram3_q <= ram3[ram_index];
    end
    wire [31:0] ram_rdata = {ram3_q, ram2_q, ram1_q, ram0_q};

    // -----------------------------------------------------------------
    // UART: TXDATA holds one byte until the transmitter takes it.
    // -----------------------------------------------------------------
    reg  [7:0] tx_byte;
    reg        tx_pending;
    wire       tx_ready;

    // Word offset 0 within the UART region is TXDATA, 1 is STATUS.
    wire uart_txdata = sel_uart & (dmem_addr[3:2] == 2'd0);
    wire uart_status = sel_uart & (dmem_addr[3:2] == 2'd1);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            tx_byte    <= 8'd0;
            tx_pending <= 1'b0;
        end else if (dmem_write & uart_txdata & dmem_be[0]) begin
            tx_byte    <= dmem_wdata[7:0];
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
    assign dmem_rdata = sel_rom                ? rom_rdata :
                        sel_ram                ? ram_rdata :
                        uart_status            ? {31'd0, tx_free} :
                                                 32'd0;
endmodule
