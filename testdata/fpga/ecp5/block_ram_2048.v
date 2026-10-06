// A 2 KiB writable memory and a 2 KiB ROM, both of which infer a block
// RAM on an ECP5, which is what this file is for.
//
// Until `src/fpga/trellis` modelled a `DP16KD` neither of these could be
// placed: the device file declared the primitive, the technology mapper
// emitted it, and the placer then said *"the design needs 2 `bram` site(s)
// and the part has 0"*. Every memory on this board therefore had to be
// built out of lookup tables — `examples/mos6502_monitor` uses one lookup
// table per **byte** of a 512-byte ROM and says so — or out of the 16-word
// distributed RAMs a slice offers.
//
// Three things about the shape of the Verilog matter, and all three are
// about which cell the design becomes rather than about what it does:
//
//   * **the memory read is registered with no reset.** `q <= mem[a]` inside
//     a plain `always @(posedge clk)` is what `src/synth/proc` turns into a
//     *clocked* read port, and a clocked read port is the only kind a block
//     RAM can serve: `fpga::primitives` declines a memory with an
//     asynchronous read port, because a block whose read is combinational
//     would read a cycle early and nothing structural would notice. Add a
//     reset to that register and the promotion does not happen and the
//     memory lands in distributed RAM instead, which is exactly what
//     `ip/fifo_sync` does and why it has no block RAM in it.
//   * **the ROM has contents and no write port**, which is the case a
//     distributed RAM cannot serve at all on this flow —
//     `fpga::primitives` refuses to lower a memory with initial contents
//     onto a `TRELLIS_DPR16X4` — and the only case in which a block RAM's
//     `INITVAL` parameters reach the bitstream. `docs/fpga-trellis.md` says
//     plainly what the ordering of those contents rests on, which is not a
//     vendor bitstream: all 53 block RAMs of this board's own gateware are
//     empty.
//   * **both outputs are registered a second time**, and that is not for
//     timing. A clock earns a global buffer by driving at least
//     `MapOptions::global_buffer_threshold` clock pins, which is eight, and
//     two block RAMs are **three** — so without the pipeline registers this
//     design's clock stays on local routing, and the ECP5 backend then
//     refuses to write a bitstream at all, because a memory clocked through
//     general routing has skew nobody has a model for and `ecppack` puts a
//     memory's clock on a global network in all 164 RAMs of this board's
//     reference bitstreams. The sixteen registers take the count to
//     nineteen. `docs/fpga-trellis.md` records that as a gap rather than a
//     feature.
//
// Both memories are 2048 words of 8 bits, which is one block each in the
// `DP16KD`'s **9-bit** mode — 2048 words of 9 bits, the widest mode
// `ecppack` used that needs no wire belonging to the block two columns
// east. That is not a coincidence of sizing: a shallower memory would tie
// on block count and the mapper breaks a tie by preferring the *widest*
// mode, and the 18-bit mode is the one this backend refuses to place next
// to another block.
//
// The pins are on top-edge balls of an LFE5U-12F in caBGA-256, in the
// order `iodb.json` lists them across the die. No board has this design on
// it; the assignment only has to be fixed so the placement a test measures
// is the same on every run.
module block_ram_2048 (
    input  wire        clk,
    input  wire        we,
    input  wire [10:0] addr,
    input  wire [7:0]  wdata,
    output wire [7:0]  rdata,
    output wire [7:0]  romdata
);

    // The writable half: one write port, one registered read port. The
    // read's register must have exactly one reader for the promotion to a
    // clocked read port to happen, which is why `mem_q` goes only to the
    // pipeline register below and not to the output as well.
    reg [7:0] mem [0:2047];
    reg [7:0] mem_q;
    always @(posedge clk) begin
        if (we) mem[addr] <= wdata;
        mem_q <= mem[addr];
    end

    // The read-only half. The contents are a function of the address so
    // that a reader of a decoded bitstream can tell which word is which,
    // and so that no word is zero: an initialisation block of all zeros is
    // what every block RAM of this board's own gateware holds, and it
    // cannot tell a right ordering from a wrong one.
    reg [7:0] rom [0:2047];
    integer i;
    initial begin
        for (i = 0; i < 2048; i = i + 1) begin
            rom[i] = (i[7:0] ^ 8'h5a) | 8'h01;
        end
    end

    reg [7:0] rom_q;
    always @(posedge clk) begin
        rom_q <= rom[addr];
    end

    // The pipeline registers, which are what give the clock enough pins to
    // earn a global buffer. See the header.
    reg [7:0] rdata_q;
    reg [7:0] romdata_q;
    always @(posedge clk) begin
        rdata_q   <= mem_q;
        romdata_q <= rom_q;
    end
    assign rdata   = rdata_q;
    assign romdata = romdata_q;

endmodule
