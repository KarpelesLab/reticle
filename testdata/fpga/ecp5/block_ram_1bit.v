// A 16384 x 1 memory, which is the only shape that reaches a `DP16KD`'s
// **1-bit** mode, and the shape that found a pin nothing drove.
//
// `src/fpga/devices/ecp5.dev` gives the block five modes. A memory one bit
// wide is the one case where the narrowest of them wins outright rather
// than by a tie-break: 16384 words of one bit is one block in the 1-bit
// mode and two in any other, so `Mapper::choose_bram` picks it on block
// count alone and no change to how ties break can steer it elsewhere.
//
// **What this caught.** In that mode the block's data ports are one bit
// wide, and `Netlist::build` used to drop the bit index from the pin role
// of any port one bit wide -- which is right for a flip-flop's `d` and for
// a block RAM's `we`, declared without an index, and wrong for `DOB`,
// whose pin is `p0_dout0`. The role named no wire, the pin went to
// `Netlist::off_fabric`, and the read data net was left with **nothing
// driving it**. Nothing downstream objected: the router is only given the
// signals that have a driver and a sink, so the routing verified, every
// set bit of the bitstream decoded back through the database, and the arcs
// were exactly the arcs the router chose. On a part the read would have
// come back as all **ones**, because an unrouted input on this family
// reads as a one -- which is what `testdata/fpga/cynthion/bram_echo.v`
// did, for 32768 bytes, on a Cynthion.
//
// So the assertion that matters here is `Netlist::driverless`, not
// anything about the image. A bitstream for this design was wrong in a way
// no count of its bits could show.
//
// The shape otherwise follows `block_ram_2048.v`: the read is registered
// with no reset and its register has exactly one reader, which is what
// makes it a clocked read port a block can serve. The eight-deep pipeline
// is there for the clock rather than for timing -- a clock earns a global
// buffer at eight clock pins and one block RAM is two, and the ECP5
// backend refuses to write a bitstream for a memory clocked through
// general routing.
//
// The pins are top-edge balls of an LFE5U-12F in caBGA-256, in the order
// `iodb.json` lists them. No board has this design on it; the assignment
// only has to be fixed so that a placement a test measures is the same on
// every run.
module block_ram_1bit (
    input  wire        clk,
    input  wire        we,
    input  wire [13:0] addr,
    input  wire        wdata,
    output wire        rdata
);

    reg mem [0:16383];
    reg mem_q;
    always @(posedge clk) begin
        if (we) mem[addr] <= wdata;
        mem_q <= mem[addr];
    end

    // The pipeline, which is what gives the clock enough pins to earn a
    // global buffer. See the header.
    reg [7:0] pipe;
    always @(posedge clk) pipe <= {pipe[6:0], mem_q};
    assign rdata = pipe[7];

endmodule
