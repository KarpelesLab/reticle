// Two block RAMs that both take the `DP16KD`'s **18-bit mode**, which is
// the one mode that makes a block cost more than itself.
//
// An 18-bit block's two top data bits of each port are the same
// interconnect wires as the two bottom bits of the block two columns east,
// so the two cannot both be used; `docs/fpga-trellis.md` has the wires and
// `TrellisFabric::configure_bram` refuses a bitstream that does it. That
// refusal had never been *shown* refusing, and the file said so: "no test
// builds an 18-bit block beside another, because nothing this flow maps
// chooses the 18-bit mode at a depth where two blocks land two columns
// apart". This design is that test, and it exists because a real one
// walked into it — `testdata/fpga/cynthion/usb_mjpeg.v` placed and routed
// for twenty-eight minutes and then wrote no bitstream, because the placer
// had never been told the rule the writer enforces.
//
// **Why these memories reach the 18-bit mode.** `Mapper::choose_bram`
// costs a mode at `blocks * (1 + sterilises)`, so at 1024 words of 18 bits
// the 18-bit mode is one block costing two and the 9-bit mode is two
// blocks costing two — a genuine tie, which the widest mode wins. Make
// either memory deeper and the tie breaks the other way and this design
// stops testing anything, which is why
// `two_eighteen_bit_blocks_are_not_placed_two_columns_apart` asserts that
// both of them really did sterilise a neighbour rather than only that the
// bitstream came out.
//
// **Why the outside is eight bits wide when the memories are eighteen.**
// Balls. Two 18-bit read ports brought out whole would be 36 pads on top
// of the address and the write data; the write data is replicated up to
// eighteen bits inside and the two reads are reduced to one bit, which
// keeps the pin list inside the top edge this family's test designs use.
// Every one of the 18 bits still has a reader — a reduction reads them
// all — so nothing narrows the memory back down behind the test's back.
//
// **Why the reads are registered twice.** The same reason
// `block_ram_2048.v` gives: `q <= mem[a]` with exactly one reader is what
// becomes a block RAM's clocked read port, and a clock earns a global
// buffer only at eight clock pins, which two block RAMs' four do not
// reach. The two 18-bit pipeline registers take it well past.
module block_ram_18bit_pair (
    input  wire       clk,
    input  wire       we,
    input  wire [9:0] addr,
    input  wire [7:0] wdata,
    output wire       rdata
);

    // Eighteen bits from eight, so that the memories are the width the
    // mode needs without the pads to match.
    wire [17:0] wide = {wdata, wdata, wdata[1:0]};

    reg [17:0] mem0 [0:1023];
    reg [17:0] mem1 [0:1023];
    reg [17:0] q0;
    reg [17:0] q1;
    always @(posedge clk) begin
        if (we) mem0[addr] <= wide;
        q0 <= mem0[addr];
    end
    always @(posedge clk) begin
        if (we) mem1[addr] <= ~wide;
        q1 <= mem1[addr];
    end

    // The pipeline, which is what gives the clock its pins.
    reg [17:0] p0;
    reg [17:0] p1;
    always @(posedge clk) begin
        p0 <= q0;
        p1 <= q1;
    end

    assign rdata = (^p0) ^ (^p1);

endmodule
