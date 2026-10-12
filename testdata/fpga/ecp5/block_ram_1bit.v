// A 16384 x 1 memory: the narrowest, deepest shape this flow has to map,
// and the one that says which `DP16KD` mode it is willing to use.
//
// **It takes one block, and that is the point.** 16384 words of one bit is
// one block in the `DP16KD`'s 1-bit mode and eight in its 9-bit mode, and
// `Mapper::choose_bram` takes the one.
//
// That needs a test because for a day it took the eight. The 4-, 2- and
// 1-bit modes were marked `unverified` and ordered behind every other
// mode, on the strength of an A/B that did not hold: the 4-bit build of
// `ip/compress/inflate` was measured on a part that had just run a
// 123-case corpus and the 9-bit build on a fresh load, and that design's
// failures depend on what decoded before them.
// `testdata/fpga/cynthion/bram_probe.v` then read all three modes back off
// a Cynthion at full depth -- 16384 x 1, 8192 x 2, 4096 x 4, one address at
// a time and then every address -- and every address landed where it was
// asked for and every value came back unpermuted. The marker is gone and
// seven blocks came back with it.
//
// **What it was written for, and catches again.** In the 1-bit mode a
// block's data ports are one bit wide, and `Netlist::build` used to drop
// the bit index from the pin role of any port one bit wide -- right for a
// flip-flop's `d` and a block RAM's `we`, declared without an index, and
// wrong for `DOB`, whose pin is `p0_dout0`. The role named no wire, the
// pin went to `Netlist::off_fabric`, and the read data net was left with
// nothing driving it; on a part it read all **ones**, which is what
// `testdata/fpga/cynthion/bram_echo.v` did for 32768 bytes. While the mode
// was quarantined this design no longer reached a one-bit port and the
// regression moved to the 7 series; it is back under test here too.
//
// What this would not catch: where a word sits in a row of the *initial*
// contents in a narrow mode. This memory has none, `bram_probe.v` writes
// at run time, and so neither reads `init low` -- which stays a reading.
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
