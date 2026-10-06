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
// Two things about the shape of the Verilog matter, and both are about
// which cell the memory becomes rather than about what it does:
//
//   * **the read is registered with no reset.** `q <= mem[a]` inside a
//     plain `always @(posedge clk)` is what `src/synth/proc` turns into a
//     *clocked* read port, and a clocked read port is the only kind a
//     block RAM can serve: `fpga::primitives` declines a memory with an
//     asynchronous read port, because a block whose read is combinational
//     would read a cycle early and nothing structural would notice. Add a
//     reset to the register and the promotion does not happen and the
//     memory lands in distributed RAM instead, which is exactly what
//     `ip/fifo_sync` does and why it has no block RAM in it.
//   * **the ROM has contents and no write port**, which is the case a
//     distributed RAM cannot serve at all on this flow — `fpga::primitives`
//     refuses to lower a memory with initial contents onto a
//     `TRELLIS_DPR16X4` — and the only case in which a block RAM's
//     `INITVAL` parameters reach the bitstream. `docs/fpga-trellis.md` says
//     plainly what the ordering of those contents rests on, which is not a
//     vendor bitstream: all 53 block RAMs of this board's own gateware are
//     empty.
//
// Both memories are 2048 words of 8 bits, which is one block each in the
// `DP16KD`'s **9-bit** mode — 2048 words of 9 bits, the one mode
// `ecppack` used for all forty-four block RAMs of `facedancer.bit`. That
// is not a coincidence of sizing: a shallower memory would tie on block
// count and the mapper breaks a tie by preferring the *widest* mode, and
// the 18-bit mode is the one this backend refuses to place next to another
// block because the two modes' top data bits are the neighbour's bottom
// ones.
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

    // The writable half: one write port, one registered read port.
    reg [7:0] mem [0:2047];
    reg [7:0] rdata_q;
    always @(posedge clk) begin
        if (we) mem[addr] <= wdata;
        rdata_q <= mem[addr];
    end
    assign rdata = rdata_q;

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

    reg [7:0] romdata_q;
    always @(posedge clk) begin
        romdata_q <= rom[addr];
    end
    assign romdata = romdata_q;

endmodule
