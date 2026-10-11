// A 16384 x 1 memory: the narrowest, deepest shape this flow has to map,
// and the one that says which `DP16KD` mode it is willing to use.
//
// **It takes eight blocks, and that is the point.** 16384 words of one bit
// is one block in the `DP16KD`'s 1-bit mode and eight in its 9-bit mode,
// and `Mapper::choose_bram` takes the eight. The 1-bit mode is marked
// `unverified` in `src/fpga/devices/ecp5.dev`: where the 4-, 2- and 1-bit
// words sit in an 18 kbit array is a reading nothing has measured, and the
// 4-bit mode beside it is measured **wrong** -- `ip/compress/inflate`'s
// 320 x 4 code-length memory decoded dynamic-Huffman streams into
// `80 | E_DIST` on a Cynthion there and decodes the whole corpus in the
// 9-bit mode. So `choose_bram` orders a confirmed layout ahead of an
// unverified one before it counts anything, and this file is what pins that
// it does. Seven blocks of waste is the price of a memory that reads back.
//
// **What it was written for, and no longer catches.** In the 1-bit mode a
// block's data ports are one bit wide, and `Netlist::build` used to drop
// the bit index from the pin role of any port one bit wide -- right for a
// flip-flop's `d` and a block RAM's `we`, declared without an index, and
// wrong for `DOB`, whose pin is `p0_dout0`. The role named no wire, the
// pin went to `Netlist::off_fabric`, and the read data net was left with
// nothing driving it; on a part it read all **ones**, which is what
// `testdata/fpga/cynthion/bram_echo.v` did for 32768 bytes. Quarantining
// the mode means this design no longer reaches a one-bit port, so the
// regression test for that fix is
// `tests/fpga_xray_bram.rs::a_one_bit_wide_memorys_data_pin_reaches_the_fabric`,
// on the family whose 1-bit mode is not in question.
//
// `Netlist::driverless` is still asserted here, because it costs nothing
// and because a bitstream for this design was once wrong in a way no count
// of its bits could show.
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
